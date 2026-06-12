# SunlightOS System Information Improvements (Phase 5.12)

## Objective
Enhance system information reporting to match or exceed Lux kernel capabilities, providing real-time memory, CPU, and process statistics to users.

## Current State Analysis

### Lux Kernel Approach
```c
typedef struct {
    uint64_t highestPhysicalAddress;
    uint64_t lowestUsableAddress;
    uint64_t highestUsableAddress;
    size_t highestPage;
    size_t usablePages, usedPages;      // Pages, not bytes
    size_t reservedPages;
} PhysicalMemoryStatus;

// Exposed via pmmStatus() kernel API
void pmmStatus(PhysicalMemoryStatus *dst);
```

### SunlightOS Current State
- PMM has: `stats()` → `(total_frames, free_frames)`
- sysfetch.rs: **Hardcoded values** (240MB used, 256MB total, 1337s uptime)
- No real-time system stats available to userspace
- No CPU info, process info, or memory breakdown

## Improvement Strategy

### Phase 1: Kernel-Side Enhancements (Minimal)

#### 1.1 Enhance PMM Stats Structure
**File**: `kernel/src/memory/pmm.rs`

```rust
#[derive(Clone, Copy)]
pub struct MemoryStats {
    pub total_pages: usize,      // All usable pages
    pub free_pages: usize,       // Currently free
    pub used_pages: usize,       // Currently allocated
    pub reserved_pages: usize,   // Reserved for kernel
}

impl PhysicalMemoryManager {
    pub fn detailed_stats(&self) -> MemoryStats {
        // Calculate from bitmap
    }
}
```

#### 1.2 Create System Stats Syscall (Optional - Phase 5.12B)
**File**: `kernel/src/arch/x86_64/syscall.rs`

```rust
#[repr(u64)]
pub enum SunlightSyscall {
    ...
    SysInfo = 81,    // New: Get system information
}
```

Payload:
```rust
pub struct SysInfo {
    pub uptime_ms: u64,
    pub memory_total: u64,
    pub memory_used: u64,
    pub processes: u32,
    pub cpu_count: u32,
}
```

#### 1.3 Expose Boot/Uptime Counter
**File**: `kernel/src/main.rs`

Add a kernel-wide uptime counter:
```rust
static BOOT_TIME_MS: spin::Mutex<u64> = spin::Mutex::new(0);

pub fn uptime_ms() -> u64 {
    *BOOT_TIME_MS.lock()
}
```

Increment in timer interrupt handler (exists in interrupts.rs).

### Phase 2: Userspace Improvements

#### 2.1 Enhanced sysfetch (Immediate)
**File**: `sunshell/src/sysfetch.rs`

```rust
pub fn render_sysfetch_with_stats(
    username: &str,
    mem_stats: &MemoryStats,      // From syscall or VFS
    uptime_ms: u64,               // From syscall or /proc
    out: &mut [u8],
) -> usize {
    // Real calculations instead of hardcoded values
}
```

#### 2.2 New Shell Commands

**`free` command**:
```
user@sunlightos:~$ free
              total    used    free    available
Memory:       256M    240M     16M     16M
```

**`uptime` command**:
```
user@sunlightos:~$ uptime
 HH:MM:SS up D days, HH:MM, N users
```

**`ps` command** (lightweight):
```
user@sunlightos:~$ ps
  PID  PPID  USER     CMD
    1     0  root     init
   42     1  root     vfs_server
   99     1  root     tty_server
```

#### 2.3 CPU Info Display
Get from Limine bootloader info or detect at runtime:
```
Limine provides:
- CPU count
- CPU model (optional)
- TSC frequency (for timing)
```

### Implementation Priority

#### 🟢 High Priority (Quick Win)
1. ✅ Fix sysfetch hardcoded values → Query PMM stats directly
2. Add `free` command (simple shell wrapper around memory stats)
3. Add `uptime` command (query kernel uptime counter)

#### 🟡 Medium Priority (Phase 5.12)
1. Create `/proc/meminfo` VFS node (like Linux)
2. Create `/proc/uptime` VFS node
3. Enhanced `sysfetch` with color-coded memory bars

#### 🔴 Lower Priority (Phase 5.13+)
1. Full `/proc` filesystem implementation
2. `ps` with real process list
3. CPU frequency/thermal info
4. `dmesg`, `journalctl` equivalents

## Proposed New Shell Commands

### `free` - Display memory usage

```bash
user@sunlightos:~$ free
              total    used    free    percentage
Memory:        256M   48M     208M     18%
```

**Implementation**: 1 syscall to get stats, simple math.

### `uptime` - Show system uptime

```bash
user@sunlightos:~$ uptime
 11:45:30 up 2 days, 3:45, 1 user
```

**Implementation**: Query kernel uptime counter, convert to days/hours/mins.

### `ps` - List processes (lightweight)

```bash
user@sunlightos:~$ ps
  PID  PPID  UID   CMD
    1     0    0   init
   10     1    0   vfs_server
   20     1    0   tty_server
   30     1    0   timer_server
```

**Implementation**: Query scheduler (existing), walk process list.

### Enhanced `sysfetch`

```
root@sunlightos
━━━━━━━━━━━━━━━━━━━━━━━
OS:        SunlightOS 0.1.0
Kernel:    5.11 (ACPI Phase)
Uptime:    2 days, 3 hours, 45 min
CPU:       2 cores @ 2.5 GHz (QEMU)
Memory:    ████████░░ 48/256 MB
───────────────────────
Disk:      [stat from /proc]
Shell:     sshl 0.1.0
Color:     [████████]
```

## Code Architecture Changes

### New Files
```
sunlight-utils/src/lib.rs          # Shared utility functions
├── memory.rs                        # Memory stat formatting
├── uptime.rs                        # Uptime calculation
└── process.rs                       # Process stat helpers

sunshell/src/commands/
├── free.rs                          # free command
├── uptime.rs                        # uptime command
└── ps.rs                            # ps command (future)
```

### Modified Files
```
kernel/src/main.rs                  # Add uptime counter
kernel/src/arch/x86_64/syscall.rs   # Add SysInfo syscall (optional)
sunshell/src/main.rs                # Register new commands
sunshell/src/sysfetch.rs            # Use real stats
services/vfs_server/src/main.rs     # /proc endpoints (Phase 5.13)
```

## Feasibility & Timeline

| Feature | Effort | Time | Phase |
|---------|--------|------|-------|
| Real memory in sysfetch | 1 file | 15m | Now |
| `free` command | 1 file | 20m | Now |
| `uptime` command | 1 file | 30m | Now |
| Kernel uptime counter | 1 file | 30m | 5.12 |
| `/proc/meminfo` VFS | 1 file | 1h | 5.12 |
| Enhanced ps (static) | 2 files | 1h | 5.12 |
| Full /proc FS | Multiple | N/A | 5.13+ |

## Comparison: Lux vs SunlightOS

| Feature | Lux | SunlightOS Current | SunlightOS After |
|---------|-----|-------------------|------------------|
| Memory reporting | ✅ pmmStatus() | ❌ Hardcoded | ✅ Real-time |
| Uptime tracking | ✅ (via kernel) | ❌ Hardcoded | ✅ Real-time |
| CPU info | ✅ (Limine) | ❌ None | ✅ From bootloader |
| `free` command | ❌ N/A | ❌ N/A | ✅ New |
| `uptime` command | ❌ N/A | ❌ N/A | ✅ New |
| `ps` command | ✅ Full | ❌ N/A | 🟡 Basic (Phase 5.12) |
| `/proc` FS | ✅ Full | ❌ N/A | 🟡 Partial (Phase 5.13) |

## Next Steps

1. ✅ **Immediate** (Today):
   - Modify sysfetch.rs to query real memory stats from PMM
   - Implement `free` command
   - Implement `uptime` command with hardcoded boot time

2. 🟡 **Phase 5.12**:
   - Add kernel uptime tracking
   - Create SysInfo syscall (optional)
   - Implement `/proc/meminfo` VFS endpoint

3. 🔴 **Phase 5.13+**:
   - Full `/proc` filesystem
   - Real `ps` with scheduler integration
   - CPU thermal/frequency monitoring

## References

- **Lux Kernel Memory Model**: `~/Projects/kernel/src/memory/physical.c`
- **Linux /proc spec**: https://man7.org/linux/man-pages/man5/proc.5.html
- **POSIX.1-2008 utilities**: uptime(1), free(1), ps(1)
