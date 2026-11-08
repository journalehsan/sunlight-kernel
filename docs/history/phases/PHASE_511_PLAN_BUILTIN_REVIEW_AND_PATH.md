# Phase 5.11: Builtin Commands Review & PATH Environment — PLAN

**Status:** PLANNING  
**Goal:** Debug builtin commands, implement PATH for extensible command dispatch  
**Architecture:** Microkernel approach with environment variables

---

## The Problem We're Solving

Currently:
```bash
$ help
Builtins: whoami, id, uname, useradd, userdel, passwd, ...

$ ls
sshl: command not found  ← Should work! (from sunlight-utils)

$ ping
sshl: command not found  ← Should work! (from sunlight-net-utils)
```

**Why?** Commands are hardcoded as builtin matching. External binaries aren't discovered or executed.

---

## Phase 5.11 Goals (In Order)

### 1. Debug & Test Builtin Commands (Week 1)
- [ ] Create test plan for each builtin
- [ ] Test in QEMU with `./tools/run.sh --build`
- [ ] Verify no caching issues
- [ ] Document working status of each command

### 2. Review sunlight-utils & sunlight-net-utils (Week 1)
- [ ] Check if binaries are being built
- [ ] Verify they're included in ISO
- [ ] Check if shell can execute them

### 3. Implement Environment Variables (Week 2)
- [ ] Add environ array to shell state
- [ ] Set initial variables (PATH, HOME, USER, etc.)
- [ ] Implement `echo $VAR` variable expansion
- [ ] Implement `export VAR=value`

### 4. Implement PATH Lookup (Week 2)
- [ ] Parse PATH environment variable
- [ ] Search directories for executables
- [ ] Execute external binaries
- [ ] Fall back to builtins if not found

### 5. Integration Testing (Week 3)
- [ ] Test builtin + external commands mix
- [ ] Test variable substitution
- [ ] Test PATH updates
- [ ] Verify performance

---

## Architecture: Microkernel Command Dispatch

```
User types: $ ls /tmp
    ↓
Shell receives input
    ↓
Parse command line
    ↓
Is it a builtin? (hardcoded match)
    ├─ YES → Execute builtin function directly
    │
    └─ NO → Search PATH for executable
        ├─ Found → Execute via spawn IPC
        └─ Not found → "command not found" error
```

### Shell State Extensions

```rust
struct Shell {
    // ... existing fields ...
    
    // Environment variables (new)
    environ: [u8; 2048],          // Raw env string: "VAR=value\0VAR2=value2\0"
    environ_len: usize,
    
    // Variable cache (optimization)
    path_var: [u8; 512],          // Cached PATH value
    path_var_len: usize,
    home_var: [u8; 256],          // Cached HOME value
    home_var_len: usize,
    
    // Process state
    pwd: [u8; 256],               // Current working directory
    pwd_len: usize,
}
```

---

## Phase 5.11A: Builtin Commands Testing

### Test Matrix

| Command | Status | Notes | Priority |
|---------|--------|-------|----------|
| **whoami** | ✅ | Should work, shows logged-in user | P0 |
| **id** | ✅ | Should work, shows uid/gid | P0 |
| **uname** | ✅ | Should work, static info | P0 |
| **echo** | ✅ | Should work, prints args | P0 |
| **pwd** | ⏳ | Needs working DIR tracking | P1 |
| **cd** | ⏳ | Needs DIR state management | P1 |
| **cat** | ✅ | Should work via VFS IPC | P0 |
| **clear** | ✅ | Should work, returns nothing | P0 |
| **useradd** | ⏳ | Need to verify user creation | P1 |
| **userdel** | ⏳ | Need to verify user deletion | P1 |
| **passwd** | ✅ | Just implemented, should work | P0 |
| **chmod** | ⏳ | VFS needs chmod support | P2 |
| **chown** | ⏳ | VFS needs chown support | P2 |
| **groups** | ⏳ | Hardcoded, needs lookup | P1 |
| **help** | ✅ | Just updated, should work | P0 |
| **exit** | ✅ | Should work, returns exit code | P0 |

### Test 1: Identity Commands

```bash
$ whoami
alice         ← Should show logged-in user

$ id
uid=1001(alice) gid=100(users)  ← Should show correct uid/gid

$ uname -a
SunlightOS sunlight 0.1.0 x86_64  ← Should show system info
```

**Expected:** All 3 commands work correctly for logged-in users.

### Test 2: I/O Commands

```bash
$ echo "Hello, SunlightOS"
Hello, SunlightOS    ← Should echo arguments

$ cat /etc/motd
Welcome to SunlightOS\n  ← Should read via VFS

$ clear
(screen clears)      ← Should work (returns empty)
```

**Expected:** I/O operations work without errors.

### Test 3: User Management

```bash
# As root
$ useradd testuser
OK          ← Should create user

$ grep testuser /etc/passwd
testuser:x:1001:100::/home/testuser:/bin/sh  ← User created

$ passwd testuser
New password: ***
passwd: password updated  ← Should set password

$ userdel testuser
OK         ← Should delete user
```

**Expected:** User lifecycle works (create → password → delete).

### Test 4: Path & Navigation

```bash
$ pwd
/root        ← Should show current directory

$ cd /tmp
$ pwd
/tmp         ← Should change directory

$ cd /home
$ pwd
/home        ← Should navigate
```

**Expected:** Directory state tracking works.

---

## Phase 5.11B: External Binaries Discovery

### Check 1: Verify Binaries Are Built

```bash
ls -la target/x86_64-unknown-none/debug/sunlight-utils
ls -la target/x86_64-unknown-none/debug/sunlight-net-utils
```

**Expected:** Both binaries exist and are executable.

### Check 2: Verify Binaries Are in ISO

```bash
# Mount ISO and check
file target/sunlightos.iso
# Extract and verify binaries are present
```

**Expected:** Binaries are packaged in the ISO.

### Check 3: Check Availability in Shell

```bash
# This requires PATH implementation (Phase 5.11C)
$ which ls
/bin/ls         ← Should find binary

$ ls /tmp
(lists /tmp)    ← Should execute sunlight-utils ls command
```

**Expected:** External binaries are discoverable and executable.

---

## Phase 5.11C: Environment Variables

### Variables to Support

| Variable | Purpose | Initial Value | Writable |
|----------|---------|----------------|----------|
| **PATH** | Command search directories | `/bin:/usr/bin:/usr/local/bin` | ✅ Yes |
| **HOME** | User home directory | `/root` (or user's home) | ✅ Yes |
| **USER** | Current username | logged-in user | ⚠️ No |
| **UID** | Current user ID | 1001 | ⚠️ No |
| **SHELL** | Current shell | `/bin/sh` | ✅ Yes |
| **PWD** | Current working directory | `/` | 🔄 Auto-update |
| **OLDPWD** | Previous directory | (empty) | 🔄 Auto-update |
| **TERM** | Terminal type | `xterm` | ✅ Yes |

### Implementation: Variable Expansion

```bash
# User types:
$ echo $HOME
/root              ← Variable substituted

$ echo $USER
alice              ← Variable substituted

$ export DEBUG=1
$ echo $DEBUG
1                  ← Custom variable

$ PATH=/custom/bin:$PATH
$ export PATH
(new PATH set)     ← PATH modification
```

### Variable Substitution Algorithm

```rust
fn expand_variables(line: &str, environ: &[u8]) -> String {
    let mut result = String::new();
    let mut chars = line.chars().peekable();
    
    while let Some(ch) = chars.next() {
        if ch == '$' {
            // Parse variable name
            let mut var_name = String::new();
            while let Some(&next_ch) = chars.peek() {
                if next_ch.is_alphanumeric() || next_ch == '_' {
                    var_name.push(chars.next().unwrap());
                } else {
                    break;
                }
            }
            
            // Look up variable in environ
            if let Some(value) = get_environ_var(environ, &var_name) {
                result.push_str(value);
            }
        } else {
            result.push(ch);
        }
    }
    
    result
}
```

---

## Phase 5.11D: PATH Lookup & Binary Execution

### Command Resolution Algorithm

```
User types: $ ls -la /tmp
    ↓
Parse: cmd="ls", args=["-la", "/tmp"]
    ↓
Check if "ls" is builtin?
    ├─ YES: Execute builtin_ls("-la", "/tmp")
    ├─ NO: Search PATH
    │   ├─ Try /bin/ls
    │   ├─ Try /usr/bin/ls
    │   ├─ Try /usr/local/bin/ls
    │   ├─ FOUND: Execute /bin/ls via spawn IPC
    │   └─ NOT FOUND: Error "ls: command not found"
    └─ Return exit code
```

### PATH Search Implementation

```rust
fn resolve_command(cmd: &str, path_var: &[u8]) -> Option<String> {
    // Parse PATH directories
    let path_str = core::str::from_utf8(path_var).unwrap_or("");
    
    for dir in path_str.split(':') {
        let full_path = format!("{}/{}", dir, cmd);
        
        // Check if file exists via VFS
        if vfs_stat(&full_path).is_ok() {
            return Some(full_path);
        }
    }
    
    None
}

fn execute_external(path: &str, args: &[&str]) -> Result<i32, Error> {
    // Use spawn IPC to execute external binary
    // Similar to how useradd spawns processes
    let spawn_msg = IpcMsg::with_label(SpawnMsg::SPAWN)
        .word(0, encode_path(path))
        .word(4, self.uid as u64)
        .word(5, self.gid as u64);
    
    let reply = ipc_call(spawn_cap, spawn_msg);
    // Wait for process and return exit code
}
```

---

## Testing Plan

### Phase 5.11A: Builtin Commands (1 week)

**Day 1-2: Test Plan Creation**
- [ ] Document each command's expected behavior
- [ ] Create test scripts for each command
- [ ] Identify which commands need VFS support

**Day 3-4: Manual Testing in QEMU**
- [ ] Build with `cargo build --workspace`
- [ ] Run `./tools/run.sh --build`
- [ ] Test each command from test plan
- [ ] Document failures and root causes

**Day 5: Fix Issues**
- [ ] Fix failing commands
- [ ] Re-test all commands
- [ ] Verify no regressions

### Phase 5.11B: Binary Discovery (3 days)

**Day 1: Build & Package Check**
- [ ] Verify sunlight-utils builds
- [ ] Verify sunlight-net-utils builds
- [ ] Check ISO contents

**Day 2-3: Shell Integration Check**
- [ ] Add code to detect external binaries
- [ ] Create discovery test script
- [ ] Document findings

### Phase 5.11C: Environment Variables (1 week)

**Day 1-2: Variable Storage**
- [ ] Add environ field to Shell
- [ ] Initialize standard variables
- [ ] Implement `get_environ_var()`

**Day 3-4: Variable Expansion**
- [ ] Implement `expand_variables()`
- [ ] Test `echo $HOME`, `echo $USER`, etc.
- [ ] Handle undefined variables

**Day 5: Export Command**
- [ ] Implement `export VAR=value`
- [ ] Update environ array
- [ ] Test variable persistence

### Phase 5.11D: PATH & Binary Execution (1 week)

**Day 1-2: PATH Parsing**
- [ ] Parse PATH variable
- [ ] Implement `resolve_command()`
- [ ] Test directory search

**Day 3-4: Binary Execution**
- [ ] Connect to spawn IPC
- [ ] Implement `execute_external()`
- [ ] Test external binary execution

**Day 5: Integration & Testing**
- [ ] Test builtin + external mix
- [ ] Test PATH modifications
- [ ] Performance testing

---

## Debugging & Caching Issues

### The Problem You Identified

When you run `./tools/run.sh --build`, sometimes old text shows (e.g., "passwd: not implemented").

**Possible causes:**
1. **ISO caching** — Old ISO still being used
2. **QEMU disk caching** — Kernel/services not reloaded
3. **Rebuild not complete** — Some packages cached

### Solution: Clean Build

```bash
# Complete clean rebuild
cargo clean
cargo build --workspace
./tools/run.sh --build --no-cache  # Force fresh QEMU
```

### Verify Fresh Build

```bash
# Check timestamps
ls -la target/sunlightos.iso
ls -la target/x86_64-unknown-none/debug/sunlight-kernel

# Should both be very recent (within seconds of each other)
```

---

## Code Structure for Phase 5.11

### File Changes Needed

**1. sunshell/src/main.rs** — Core shell enhancements
```rust
// Add to Shell struct
environ: [u8; 2048],
path_var: [u8; 512],
home_var: [u8; 256],
pwd: [u8; 256],

// Add functions
fn get_environ_var(&self, name: &str) -> Option<&str>
fn set_environ_var(&mut self, name: &str, value: &str)
fn expand_variables(&self, line: &str) -> String
fn resolve_command(&self, cmd: &str) -> Option<String>
fn execute_external(&mut self, path: &str, args: &[&str]) -> ([u8; MAX_OUT], usize)
```

**2. Services** — No changes needed (spawn already works)

**3. Kernel** — No changes needed (already handles external process spawning)

---

## Success Criteria

### Phase 5.11A: ✅ Builtin Commands All Working
- [ ] All P0 commands tested and working
- [ ] All P1 commands tested (may defer P2)
- [ ] No regressions in user management

### Phase 5.11B: ✅ Binaries Available in ISO
- [ ] sunlight-utils binary exists
- [ ] sunlight-net-utils binary exists
- [ ] Both are executable

### Phase 5.11C: ✅ Environment Variables Work
- [ ] Standard variables set at shell start
- [ ] Variable expansion in commands
- [ ] `export` command works
- [ ] PATH can be modified

### Phase 5.11D: ✅ External Commands Execute
- [ ] `ls` works (via sunlight-utils)
- [ ] `ping` works (via sunlight-net-utils)
- [ ] Commands in PATH are discovered
- [ ] Fallback to builtins if not in PATH

---

## Phase 5.11E: Shell Loading Refactor (Microkernel Proper)

**NOTE:** This is the proper microkernel architecture, to be done after shell is working in Phase 5.11A-D.

### Current Issue (Technical Debt)

Shell is embedded in kernel ELF with kernel address space:
```
[ELF] PT_LOAD vaddr=ffffffff80000000  ← Kernel addresses!
```

Should be user-space addresses:
```
[ELF] PT_LOAD vaddr=0x400000  ← User-space addresses
```

### Solution: Load from RamFs

```rust
// Phase 5.11E: Replace embedded bytes with RamFs loading

// Before (kernel/src/main.rs):
static SUNSHELL_ELF_BYTES: &[u8] = include_bytes!("sshl");

// After: Shell in RamFs
kernel/initialize_ramfs() {
    ramfs.add_file("/bin/sshl", sunshell_elf_bytes);
}

// Then in spawn.rs:
fn spawn_from_path() {
    let bytes = ramfs.read("/bin/sshl")?;  // From kernel-internal RamFs
    load_elf(bytes, ...);
}
```

### Why This Is Better

✅ **Microkernel principle** — Kernel doesn't embed user binaries  
✅ **User-space addresses** — Shell loads at 0x400000+  
✅ **Scalable** — Can swap shells without kernel rebuild  
✅ **Standard** — Matches Unix bootloader model  

### Implementation Plan (Phase 5.11E)

**Step 1: Fix linker target** (quick, may solve immediately)
```bash
# Add --target flag to force x86_64-unknown-none
cargo build --package sunshell --release --features sunlight \
  --no-default-features \
  --target x86_64-unknown-none
```

**Step 2: If linker target doesn't work, implement RamFs loading**
- Extract shell ELF from embedded bytes
- Add to RamFs at kernel init
- Change spawn.rs to read from RamFs
- Test user-space addresses

**Step 3: Validate**
- Verify ELF loads at 0x400000+
- Verify shell works
- Verify no kernel panic

### Timeline

- **Phase 5.11A-D:** Get shell working (current plan)
- **Phase 5.11E:** Proper microkernel refactor (after shell stable)

## Future: Phase 5.12+

Once Phase 5.11 is complete:

**Phase 5.12: Shell Extensions**
- [ ] Implement pipes (|)
- [ ] Implement redirections (>, <, >>)
- [ ] Implement wildcards (*, ?)

**Phase 5.13: Advanced Features**
- [ ] History/readline
- [ ] Aliases
- [ ] Functions
- [ ] Conditional execution (&&, ||)

**Phase 6.0: Production Shell**
- [ ] Full POSIX compatibility
- [ ] Proper error handling
- [ ] Performance optimization

---

## Why This Approach?

### Microkernel Benefits

✅ **Extensible** — New commands don't need code changes  
✅ **Modular** — Builtin vs. external clearly separated  
✅ **Flexible** — Users can add/remove commands  
✅ **Standard** — Matches Unix/Linux model  

### Environment Variables Benefits

✅ **Configurable** — System behavior via variables  
✅ **Portable** — Standard across Unix systems  
✅ **Scriptable** — Commands can use $VAR  
✅ **Future-proof** — Extensible mechanism  

---

## References

- **Current shell:** sunshell/src/main.rs (1300+ lines)
- **Builtin commands:** All in sunshell/src/main.rs
- **External binaries:** sunlight-utils, sunlight-net-utils
- **Unix PATH standard:** https://pubs.opengroup.org/onlinepubs/9699919799/basedefs/V1_chap08.html
- **Environment variables:** https://pubs.opengroup.org/onlinepubs/9699919799/basedefs/V1_chap08.html#tag_08_03

---

## Summary

**Phase 5.11 transforms the shell from a hardcoded builtin dispatcher into a true Unix shell:**

1. **Debug builtins** — Ensure all commands work
2. **Discover binaries** — Find external commands
3. **Add environment** — Support $VAR expansion
4. **Implement PATH** — Microkernel command dispatch

**Result:** A shell that can run both builtin commands and external utilities, with proper Unix semantics.
