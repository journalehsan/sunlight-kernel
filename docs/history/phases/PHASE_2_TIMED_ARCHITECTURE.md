# Phase 2: User-Space Time Daemon (`timed`) - Architecture & Design

## Overview

Phase 2 implements a **minimal-kernel, maximum-userland** time architecture where:
- **Kernel** exposes only raw UTC via RTC syscall
- **Userland (`timed`)** handles all formatting, timezone offsets, DST, NTP sync
- **Shell & utilities** query time state from shared state file/memory

---

## 1. Kernel Isolation Layer

### New Syscall: `SysGetTimeUtc`

**Location:** `kernel/src/arch/x86_64/syscall.rs`

Add syscall number and handler:
```rust
pub enum SunlightSyscall {
    // ... existing ...
    GetTimeUtc = 50,  // Get current UTC time as Unix Epoch seconds
}
```

**Implementation:**
- Read system RTC (via ACPI FADT or x86 CMOS)
- Return as u64 (Unix Epoch seconds since 1970-01-01T00:00:00Z)
- No formatting, no offsets, no strings
- Return 0 if RTC unavailable (will be fixed in Phase 2.2)

**ABI:**
```
Syscall: SysGetTimeUtc
Input: rax = 50
Output: rax = Unix timestamp (u64)
```

### Current RTC Availability

Check `/etc/config/rtc.conf` or `/proc/rtc` (if exists) to detect RTC:
- ACPI: FADT.boot_flags.rtc_valid_bit
- CMOS: x86 port 0x70/0x71 (already implemented in some bootloaders)
- Fallback: hardcoded boot time + tick count

---

## 2. The `timed` Daemon

### Directory Structure

```
/services/timed/
├── Cargo.toml
└── src/
    ├── main.rs           (daemon loop, IPC handler)
    ├── config.rs         (timezone/DST parsing)
    ├── state.rs          (time state management)
    ├── ntp.rs            (NTP integration - scaffold)
    └── localtime.rs      (symlink resolution, file I/O)
```

### 2.1 Timezone Configuration File Format

**Location:** `/etc/localtime` (symlink)
**Target:** `/etc/timezones/[Continent]/[City]`

**File Format (2 lines):**
```
Line 1: Offset as float or integer
  Examples: +4.5, -5, 8.0, +12.75
  
Line 2: DST flag
  0 or missing: DST inactive
  1: DST active
```

**Example: Tehran timezone**
```
/etc/timezones/Asia/Tehran:
4.5
0
```

**Example: US Eastern (EST)**
```
/etc/timezones/America/New_York:
-5
0
```

**Example: With DST active**
```
/etc/timezones/Europe/London:
0
1
```

### 2.2 Offset Calculation Function

**Safe floating-point parsing with strict type handling:**

```rust
/// Parse timezone offset string (may be float or int)
/// Returns offset in seconds
fn parse_offset_string(s: &str) -> Result<i32, &'static str> {
    let trimmed = s.trim();
    
    // Try parsing as integer first
    if let Ok(int_val) = trimmed.parse::<i32>() {
        return Ok(int_val * 3600);  // Convert hours to seconds
    }
    
    // Try parsing as float, then convert to int
    if let Ok(float_val) = trimmed.parse::<f64>() {
        let seconds = (float_val * 3600.0) as i32;
        return Ok(seconds);
    }
    
    Err("Invalid offset format")
}
```

**DST application:**
```rust
fn calculate_local_offset(base_offset: i32, dst_active: bool) -> i32 {
    if dst_active {
        base_offset + 3600  // Add 1 hour for DST
    } else {
        base_offset
    }
}
```

### 2.3 Time State Structure

**Shared state (updated by `timed`, read by userland):**

```rust
#[repr(C)]
pub struct TimeState {
    pub utc_epoch: u64,              // Unix timestamp from RTC
    pub local_offset_secs: i32,      // Offset from UTC in seconds
    pub dst_active: bool,            // Is DST currently active
    pub timezone_name: [u8; 64],     // e.g., "Asia/Tehran"
    pub timezone_name_len: usize,
    pub ntp_synced: bool,            // Has NTP synchronized the clock?
    pub ntp_drift_ppm: i32,          // PPM drift correction (parts per million)
}
```

**Size:** ~140 bytes

**Location:** `/var/run/time_state.bin` (binary)
or JSON format at `/var/run/time_state.json`

---

## 3. Daemon Loop (`main.rs`)

### Startup Sequence

1. Parse command-line args or config
2. Resolve `/etc/localtime` symlink
3. Read timezone offset + DST flag
4. Create IPC endpoint (register as "timed" with init nameserver)
5. Query kernel RTC via new `SysGetTimeUtc()` syscall
6. Initialize `TimeState`
7. Write state to `/var/run/time_state.json`
8. Enter main loop

### Main Loop

**Pseudo-code:**
```rust
loop {
    // Option 1: Wait for IPC query (blocking)
    let msg = ipc_recv(ep);
    
    match msg.label {
        TimeMsg::GET_TIME => {
            // Query kernel RTC
            let utc = sys_get_time_utc();
            
            // Apply local offset + DST
            let local_time = utc as i64 + self.local_offset_secs as i64;
            
            // Reply with TimeState
            let reply = IpcMsg::with_label(TimeMsg::REPLY)
                .word(0, utc)
                .word(1, self.local_offset_secs as u64)
                .word(2, self.dst_active as u64);
            ipc_reply_and_wait(ep, reply);
        }
        TimeMsg::SET_TIMEZONE => {
            // Reparse timezone config
            self.reload_timezone();
            ipc_reply_and_wait(ep, IpcMsg::with_label(TimeMsg::REPLY));
        }
        _ => {}
    }
    
    // Option 2: Periodic NTP sync (if network available)
    // - Every 3600 seconds: poll NTP pools
    // - Compute drift correction
    // - Update ntp_synced flag
}
```

### IPC Message Types (new)

```rust
pub mod TimeMsg {
    pub const GET_TIME: u64 = 1;          // Query current time
    pub const SET_TIMEZONE: u64 = 2;      // Reload timezone config
    pub const GET_STATE: u64 = 3;         // Get full TimeState
    pub const SYNC_NTP: u64 = 4;          // Trigger NTP sync
    pub const REPLY: u64 = 100;
    pub const ERROR: u64 = 101;
}
```

---

## 4. Configuration Parsing (`config.rs`)

### Responsibilities

- Symlink resolution: `/etc/localtime` → `/etc/timezones/...`
- File I/O (via VFS syscalls or direct file reading)
- Safe line parsing (handle missing lines, whitespace)
- Type conversion (string → f64 → i32)

### Functions

```rust
pub fn read_timezone_offset(tz_path: &str) -> Result<(i32, bool), &'static str> {
    // 1. Resolve symlink if needed
    let resolved = resolve_symlink(tz_path)?;
    
    // 2. Read file (2 lines)
    let content = read_file(&resolved)?;
    let lines: Vec<&str> = content.lines().collect();
    
    // 3. Parse offset (line 1)
    let offset_secs = parse_offset_string(lines[0])?;
    
    // 4. Parse DST (line 2, optional)
    let dst_active = if lines.len() > 1 {
        lines[1].trim() == "1"
    } else {
        false
    };
    
    Ok((offset_secs, dst_active))
}
```

---

## 5. State Management (`state.rs`)

### TimeState Persistence

**JSON Format (human-readable):**
```json
{
  "utc_epoch": 1686000000,
  "local_offset_secs": 16200,
  "dst_active": false,
  "timezone_name": "Asia/Tehran",
  "ntp_synced": false,
  "ntp_drift_ppm": 0
}
```

**Binary Format (fast I/O):**
```
[0:7]    utc_epoch: u64
[8:11]   local_offset_secs: i32
[12]     dst_active: u8 (0/1)
[13:76]  timezone_name: [u8; 64]
[77:80]  timezone_name_len: u32
[81]     ntp_synced: u8 (0/1)
[82:85]  ntp_drift_ppm: i32
```

### Write Frequency

- On startup: write initial state
- On timezone change (SET_TIMEZONE): rewrite
- On NTP sync: update ntp_synced + ntp_drift_ppm
- Option: periodically (every 60s) for clock drift tracking

---

## 6. NTP Integration (Scaffold) (`ntp.rs`)

### Phase 2.1 Scope (minimal)

**Not yet:** Full NTP client implementation
**Yes:** Scaffold structure for Phase 2.2

**Placeholder:**
```rust
pub fn ntp_poll() -> Result<DriftCorrection, NtpError> {
    // Phase 2.2: Implement NTP polling
    // - Create UDP socket (via sunlight-net)
    // - Send NTP request to pool.ntp.org
    // - Parse NTP response
    // - Compute drift (current_time - received_time)
    // - Return correction factor
    
    Ok(DriftCorrection {
        drift_ppm: 0,  // Placeholder
        sync_time: 0,
    })
}
```

---

## 7. Userland Integration

### Shell Integration (sysfetch, prompts)

**Before:**
```bash
$ date
# Calls heavy kernel syscall, formats in shell
```

**After:**
```bash
$ date
# Reads /var/run/time_state.json
# Applies offset locally, formats
# 100x faster (no IPC round-trip)
```

### Example: Shell Prompt with Time

```rust
// In sunshell/src/main.rs
let state = read_time_state("/var/run/time_state.json")?;
let local_time = state.utc_epoch as i64 + state.local_offset_secs as i64;
let hour = (local_time % 86400) / 3600;
let minute = (local_time % 3600) / 60;

println!("[{}:{}] user@host:/$ ", hour, minute);
```

---

## 8. Security & Error Handling

### Bounds Checking

✅ Offset clamping: `-12..+14` hours
✅ DST flag: 0 or 1 only
✅ Timezone name: max 64 bytes
✅ Symlink recursion limit: max 5 hops

### Error Paths

```rust
pub enum TimeError {
    RtcUnavailable,          // Kernel RTC not present
    TimezoneNotFound,        // /etc/localtime missing
    InvalidOffset,           // Offset parse failed
    IoError(&'static str),   // VFS I/O error
    SymlinkLoop,             // Circular symlink detected
}
```

---

## 9. Build & Integration

### Cargo.toml
```toml
[package]
name = "sunlight-timed"
version = "0.1.0"
edition = "2021"

[dependencies]
sunlight-ipc = { path = "../../ipc" }

[profile.release]
opt-level = 3
lto = true
strip = true
```

### Build Command
```bash
cargo build --target x86_64-unknown-none -p sunlight-timed --release
```

### Integration with Bootloader
- Add to `run.sh`: Copy `timed` binary to initrd/rootfs
- Register in `init.rs`: Start `timed` as a named service
- Ensure it runs BEFORE shell (`sunshell`) starts

---

## 10. Testing & Validation

### Unit Tests (no_std compatible)

```rust
#[test]
fn test_parse_offset_positive_float() {
    assert_eq!(parse_offset_string("+4.5"), Ok(16200));
}

#[test]
fn test_parse_offset_negative_int() {
    assert_eq!(parse_offset_string("-5"), Ok(-18000));
}

#[test]
fn test_dst_addition() {
    assert_eq!(calculate_local_offset(16200, true), 19800);
    assert_eq!(calculate_local_offset(16200, false), 16200);
}

#[test]
fn test_parse_missing_dst_line() {
    // Should default to false
    assert_eq!(parse_dst_string(""), Ok(false));
}
```

### Integration Tests

1. **Kernel RTC Check**
   - Boot system
   - Call `sys_get_time_utc()`
   - Verify non-zero timestamp

2. **Timezone Loading**
   - Create `/etc/localtime` symlink
   - Start `timed`
   - Check `/var/run/time_state.json` contains correct offset

3. **Shell Time Display**
   - Start shell
   - Run `date` equivalent
   - Verify time is offset correctly

---

## 11. Phase 2 Roadmap

| Phase | Task | Dependency |
|-------|------|------------|
| 2.0 | Design & scaffold (this doc) | - |
| 2.1 | Kernel RTC syscall + timed daemon | Kernel dev |
| 2.2 | Full NTP implementation + drift sync | sunlight-net |
| 2.3 | SIGWINCH-like daemon restart on timezone change | - |
| 2.4 | Advanced DST rules (transition dates) | - |
| 2.5 | Persistent clock drift file | VFS |
| 2.6 | Performance optimization (shared memory instead of JSON) | - |

---

## 12. Conclusion

Phase 2 delivers a **lean, secure, userland-focused time architecture** that:
- Keeps kernel minimal (one syscall)
- Handles all complexity in userspace (`timed`)
- Provides fast local time to shell/utilities
- Scaffolds NTP for later implementation
- Sets foundation for cluster-wide time sync (Phase 3)
