# passwd Command Implementation — Phase 5.10

**Status:** ✅ IMPLEMENTED & COMPILING  
**Commit:** 403721e  
**Feature:** Interactive password input, no echo, confirmation matching

---

## Overview

The `passwd` command enables users to manage passwords interactively, following Unix/Linux conventions:

```bash
$ passwd                    # Change current user's password
$ passwd alice             # Change alice's password (root only)
```

### Features

✅ **Interactive input** — User types password (character-by-character)  
✅ **No echo** — Password not displayed on terminal  
✅ **Confirmation** — Must type password twice, must match  
✅ **Permission checks** — Non-root users can only change own password  
✅ **Error handling** — Clear messages for mismatches or failures  
✅ **Shadow file updates** — Stores password in /etc/shadow  

---

## Architecture

### State Machine

The shell tracks password input mode using `PasswdState` enum:

```rust
enum PasswdState {
    None,                    // Normal command mode
    PromptNew {              // Waiting for new password
        target_user: [u8; 64],
        target_user_len: usize,
    },
    PromptConfirm {          // Waiting for confirmation
        target_user: [u8; 64],
        target_user_len: usize,
        new_password: [u8; 64],
        new_password_len: usize,
    },
}
```

### Shell Struct Extensions

```rust
struct Shell {
    // ... existing fields ...
    passwd_state: PasswdState,        // Current password input state
    passwd_buffer: [u8; 64],          // Typed password (no echo)
    passwd_buffer_len: usize,         // Length of typed password
}
```

### Input Handling Flow

```
User types: passwd
  ↓
cmd_passwd() called
  ├─ Check permissions (non-root can only change self)
  ├─ Verify target user exists
  ├─ Set passwd_state = PromptNew
  └─ Output "New password: " (prompt, no newline)
  ↓
User types password character-by-character
  ↓
handle_passwd_input() called for each byte
  ├─ Buffer characters in passwd_buffer
  ├─ DO NOT echo to terminal (silent input)
  ├─ Backspace deletes from buffer
  └─ Return (no output)
  ↓
User presses Enter
  ↓
handle_passwd_input() processes newline
  ├─ Save new_password from passwd_buffer
  ├─ Clear passwd_buffer
  ├─ Set passwd_state = PromptConfirm
  └─ Output "Retype new password: "
  ↓
User types confirmation password
  ↓
handle_passwd_input() processes newline
  ├─ Compare confirmation with new_password
  ├─ If match: call update_shadow()
  ├─ If mismatch: output "passwords do not match"
  ├─ Reset passwd_state = None
  └─ Return to normal mode
```

---

## Implementation Details

### 1. Command Invocation

```rust
// In run_line(), before parsing other commands:
if cmd_owned == "passwd" {
    return self.cmd_passwd(target_user_owned.as_deref());
}
```

**Why early handling?** Passwd needs mutable `self` to update internal state, requiring Rust borrow checker care.

### 2. cmd_passwd() Function

Initiates the password change flow:

```rust
fn cmd_passwd(&mut self, target_arg: Option<&str>) -> ([u8; MAX_OUT], usize) {
    // Determine target user
    let target_user_bytes = if let Some(arg) = target_arg {
        // Arg provided: change specified user (root only)
        if self.uid != 0 {
            return copy_out(b"passwd: permission denied\n");
        }
        arg.as_bytes()
    } else {
        // No args: change current user
        &self.username[..self.username_len]
    };

    // Verify user exists
    if !self.user_exists(target_user_bytes) {
        return copy_out(b"passwd: user not found\n");
    }

    // Enter password prompt mode
    self.passwd_state = PasswdState::PromptNew { ... };
    copy_out(b"New password: ")
}
```

**Permission Model:**
| User Type | Can Change | Result |
|-----------|-----------|--------|
| Non-root | own password | ✅ Allowed |
| Non-root | other password | ❌ Permission denied |
| Root | own password | ✅ Allowed |
| Root | any password | ✅ Allowed |

### 3. handle_passwd_input() Function

Processes character-by-character input in password mode:

```rust
fn handle_passwd_input(&mut self, byte: u8, state: PasswdState) -> ([u8; MAX_OUT], usize) {
    match byte {
        b'\n' | b'\r' => {
            // Password entry complete, process based on state
            match state {
                PasswdState::PromptNew { ... } => {
                    // Transition to confirm prompt
                    self.passwd_state = PasswdState::PromptConfirm { ... };
                    copy_out(b"Retype new password: ")
                }
                PasswdState::PromptConfirm { ... } => {
                    // Verify match, update shadow
                    if passwords_match {
                        update_shadow(...)
                    } else {
                        copy_out(b"passwd: passwords do not match\n")
                    }
                }
                PasswdState::None => { /* unreachable */ }
            }
        }
        0x08 => {
            // Backspace: delete from buffer (silently, no echo)
            if self.passwd_buffer_len > 0 {
                self.passwd_buffer_len -= 1;
            }
            ([0; MAX_OUT], 0)  // No output
        }
        c if c >= 0x20 && c <= 0x7E => {
            // Printable character: buffer it (don't echo)
            if self.passwd_buffer_len < 64 {
                self.passwd_buffer[self.passwd_buffer_len] = c;
                self.passwd_buffer_len += 1;
            }
            ([0; MAX_OUT], 0)  // No output (silent input)
        }
        _ => ([0; MAX_OUT], 0),
    }
}
```

**Key behavior:** No output for character input — this creates "silent password input" effect.

### 4. update_shadow() Function

Writes updated password to /etc/shadow:

```rust
fn update_shadow(&mut self, username: &[u8], password: &[u8]) -> ([u8; MAX_OUT], usize) {
    let vfs_cap = nameserver_lookup("vfs")?;
    let shadow_data = read_file(vfs_cap, "/etc/shadow");
    
    // Parse existing entries
    // Find target user's line, replace password field
    // Format: username:password:0:0:99999:7:::
    
    let new_shadow = /* constructed string with updated entry */;
    write_file(vfs_cap, "/etc/shadow", new_shadow.as_bytes())?;
    
    copy_out(b"passwd: password updated\n")
}
```

**Shadow format:** `username:password:0:0:99999:7:::`
- Field 1: username
- Field 2: password (plaintext in Phase 5.10, hashed in Phase 6.0)
- Fields 3-8: unused (all zeros for testing)

### 5. user_exists() Helper

Verifies target user exists before password change:

```rust
fn user_exists(&self, username: &[u8]) -> bool {
    // Send GETPWNAM IPC to VFS
    // Returns true if user found in /etc/passwd
}
```

**Prevents:** Confusing error messages by validating early.

---

## Testing the passwd Command

### Test 1: Change Own Password (Non-Root)

```bash
# First, create a new user
$ useradd alice
OK

# Then set password (as root)
$ passwd alice
New password: alice_password
Retype new password: alice_password
passwd: password updated

# Log out
$ exit

# Log in as alice with new password
Username: alice
Password: alice_password

# Verify identity
$ whoami
alice
```

### Test 2: Change Own Password (Current User)

```bash
# As alice (already logged in)
$ passwd
New password: new_alice_password
Retype new password: new_alice_password
passwd: password updated

# Logout and verify new password works
$ exit
Username: alice
Password: new_alice_password
$ whoami
alice
```

### Test 3: Permission Denial (Non-Root Changing Others)

```bash
# As non-root user (alice)
$ passwd bob
passwd: permission denied
```

### Test 4: Password Mismatch

```bash
$ passwd
New password: pass1
Retype new password: pass2
passwd: passwords do not match
```

Expected: Shell returns to prompt, password unchanged.

### Test 5: Nonexistent User (Root)

```bash
$ passwd nonexistent
passwd: user not found
```

---

## Code Structure Changes

### Modified Files

**sunshell/src/main.rs**

1. **Shell struct** — Added passwd state tracking fields
   - `passwd_state: PasswdState`
   - `passwd_buffer: [u8; 64]`
   - `passwd_buffer_len: usize`

2. **PasswdState enum** — New state machine for password modes
   - `None` — Normal command mode
   - `PromptNew` — Getting new password
   - `PromptConfirm` — Confirming password

3. **handle_byte()** function — Extended to check passwd state
   - Delegates to `handle_passwd_input()` if in password mode
   - Continues normal input handling otherwise

4. **handle_passwd_input()** — New function for password input
   - Processes bytes without echoing
   - Manages state transitions
   - Validates password confirmation

5. **cmd_passwd()** — New function to initiate password change
   - Permission checks
   - User existence validation
   - State initialization

6. **update_shadow()** — New function to persist password
   - Reads current /etc/shadow
   - Updates target user's password field
   - Writes back via VFS

7. **user_exists()** — Helper to validate user via VFS

8. **run_line()** — Restructured for mutable self handling
   - Early extraction of owned cmd/args
   - Calls mutable methods after borrowing ends

---

## Security Considerations

### Current (Testing Phase 5.10)

✅ **No echo** — Password not visible during input  
✅ **Confirmation** — Must match exactly (prevents typos)  
✅ **Permission checks** — uid=0 required to change others  
⚠️ **Plaintext storage** — Suitable for testing only  
⚠️ **No hashing** — Passwords in shadow file are plaintext  

### Next Phase (Phase 6.0)

🔒 **Password hashing** — Implement bcrypt/scrypt  
🔒 **Salt generation** — Random per-user salt  
🔒 **Shadow permissions** — Enforce mode 600 (root-only read)  
🔒 **Login rate limit** — Prevent brute force attempts  
🔒 **Account lockout** — After N failed attempts  

---

## Edge Cases Handled

| Scenario | Behavior | Code |
|----------|----------|------|
| Password too long (>64 bytes) | Silently truncate | `passwd_buffer_len < 64` check |
| Backspace at start of password | No-op, buffer_len stays 0 | `if buffer_len > 0` guard |
| Non-printable characters | Ignored | `c >= 0x20 && c <= 0x7E` check |
| Mismatch on confirmation | Reject, return to prompt | `passwords_match` comparison |
| User not in /etc/passwd | Error message | `user_exists()` validation |
| Non-root changing others | Permission denied | `self.uid != 0` check |
| VFS unavailable | Error message | `nameserver_lookup()` check |

---

## Performance

**Password entry:**
- ✅ No allocations per keystroke (buffer pre-allocated)
- ✅ O(1) per character (simple buffer append)
- ✅ No copying until final update

**Shadow file update:**
- ⚠️ Reads entire /etc/shadow (small file, acceptable)
- ⚠️ Re-parses all users (simple parser, fast)
- ⚠️ Writes entire file (atomic from shell perspective)

**Total:** Sub-millisecond for typical usage (testing phase).

---

## Integration with Existing Systems

### With VFS
- Uses `read_file()` to fetch /etc/passwd and /etc/shadow
- Uses `write_file()` to store updated /etc/shadow
- Uses `user_exists()` via GETPWNAM IPC

### With Auth Flow
- TTY calls `login.handle_key_ascii()` for credentials
- login module validates against /etc/passwd + /etc/shadow
- On success, TTY spawns shell with uid/gid
- Shell now has persistent user identity

### With Shell Commands
- `whoami` — Returns shell.username (set during login/GETPWUID)
- `id` — Returns shell.uid/gid (passed from TTY)
- `useradd` — Creates new entry in /etc/passwd
- `passwd` — Updates password in /etc/shadow

---

## Testing Checklist

- [ ] Boot system
- [ ] Login as root (root / root)
- [ ] Create new user: `useradd alice`
- [ ] Set alice's password: `passwd alice`
- [ ] Enter password twice (e.g., "mypass")
- [ ] Verify no echo (password invisible during typing)
- [ ] See "passwd: password updated"
- [ ] Exit (ctrl+d or `exit`)
- [ ] Login as alice with new password
- [ ] Verify `whoami` shows "alice"
- [ ] Test mismatch: `passwd` → type "a" then "b" at confirm → see error
- [ ] Test permission: create bob with `useradd bob`, try `passwd bob` as alice → "permission denied"
- [ ] Test nonexistent: as root, `passwd nobody` → "user not found"

---

## Known Limitations

1. **Plaintext passwords** (Phase 5.10)
   - Suitable for development/testing only
   - Will be hashed in Phase 6.0

2. **No password policy**
   - Can set empty password
   - Can set very long password (truncated at 64 bytes)
   - No complexity requirements

3. **No password aging**
   - No last-change date tracking
   - No expiration dates

4. **Immediate write**
   - No transaction/rollback if write fails
   - Could improve with staging

---

## Next Steps (Phase 5.10+)

**Immediate (Phase 5.10):**
- [ ] Test full user creation + password flow
- [ ] Test edge cases from table above
- [ ] Verify shadow file updates correctly

**Medium term (Phase 6.0):**
- [ ] Implement password hashing (bcrypt)
- [ ] Add salt generation
- [ ] Enforce shadow file permissions (mode 600)
- [ ] Add login attempt rate limiting

**Long term (Phase 6.x):**
- [ ] Password aging policies
- [ ] Password history (prevent reuse)
- [ ] Account lockout after failures
- [ ] sudo integration

---

## References

- **Unix passwd(1)** — `man passwd` (standard reference)
- **SHA-512 crypt** — Standard hashing for shadow files
- **Crypt(3)** — Password hashing functions
- **Commit 403721e** — Full implementation
- **docs/ROOT_ONLY_BOOTSTRAP.md** — User model explanation
- **sunshell/src/main.rs** — Implementation code

---

## Summary

The `passwd` command provides interactive, secure password management following Unix conventions:

- ✅ **Interactive input** with no echo
- ✅ **Confirmation matching** to prevent typos
- ✅ **Permission enforcement** (non-root can't change others)
- ✅ **Persistent storage** in /etc/shadow
- ✅ **Error handling** for edge cases

This completes the user management foundation needed for multi-user systems. Phase 6.0 will add password hashing to move from testing to production-ready security.
