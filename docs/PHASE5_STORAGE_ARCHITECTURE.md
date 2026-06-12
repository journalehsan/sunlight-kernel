# Phase 5: Storage Layout & Memory Optimization
## SunlightOS Virtual Memory & Filesystem Integration

**Status**: Current Architecture Analysis & Implementation Plan

---

## Current Architecture Overview

### 1. Memory Management Stack

```
┌─────────────────────────────────────────────────────┐
│ Kernel Memory Management (kernel/src/memory/)        │
├─────────────────────────────────────────────────────┤
│                                                      │
│  ┌──────────────────┐                               │
│  │  PMM (pmm.rs)    │ ← Physical Memory Manager      │
│  │  4 KiB frames    │                               │
│  └────────┬─────────┘                               │
│           │                                          │
│  ┌────────▼──────────┐                              │
│  │  VMM (vmm.rs)     │ ← Virtual Memory Manager      │
│  │  Page tables      │                              │
│  └────────┬──────────┘                              │
│           │                                          │
│  ┌────────▼──────────┐                              │
│  │  HEAP (heap.rs)   │ ← 1 MiB kernel heap         │
│  │  Linked allocator │                              │
│  └────────┬──────────┘                              │
│           │                                          │
│  ┌────────▼──────────┐                              │
│  │  ZRAM (zram.rs)   │ ← 512 KiB compressed pool   │
│  │  LZ4 compression  │                              │
│  └───────────────────┘                              │
│                                                      │
└─────────────────────────────────────────────────────┘
```

### 2. Filesystem Architecture

```
┌─────────────────────────────────────────────────────┐
│ VFS Server (services/vfs_server/src/main.rs)        │
├─────────────────────────────────────────────────────┤
│                                                      │
│  ┌─────────────────────────────────────────┐       │
│  │ VFS Multiplexer                         │       │
│  │ - Path routing to mounted filesystems   │       │
│  │ - Mount table management                │       │
│  │ - Handle encoding (mount + local)       │       │
│  └────────────────┬────────────────────────┘       │
│                   │                                  │
│    ┌──────────────┼──────────────┐                 │
│    │              │              │                  │
│    ▼              ▼              ▼                  │
│ ┌──────────┐  ┌──────────┐  ┌───────────┐         │
│ │ RAMFS    │  │ BootFS   │  │ SunlightFS│         │
│ │ (/)      │  │ (/boot)  │  │ (future)  │         │
│ │ INITRAMFS│  │ FAT32    │  │ BlockDev  │         │
│ └──────────┘  └──────────┘  └───────────┘         │
│    (16 KB)      (disk)        (persistent)         │
│                                                      │
└─────────────────────────────────────────────────────┘
```

### 3. FSTAB Mount Coordinator

**Current Implementation**: `services/vfs_server/src/main.rs:204-247`

```rust
fn mount_from_fstab(vfs: &mut Vfs, boot: &mut Option<BootFs>) {
    // 1. Read /etc/fstab from INITRAMFS
    // 2. Parse entries (device, mountpoint, fstype, options)
    // 3. For each entry:
    //    - If fstype == "ramfs": mount RAMFS
    //    - If fstype == "bootfs": mount BootFS
    //    - If fstype == "sunlightfs": (TBD) mount block device
    // 4. Return boot mount status
}
```

**Supported filesystems**:
- `ramfs`: INITRAMFS (read-write in memory)
- `bootfs`: FAT32 boot partition (read-only from disk)
- `sunlightfs`: SunlightFS (planned: persistent block device)

---

## ZRAM Memory Compression Integration

### Current ZRAM Implementation

**Location**: `kernel/src/memory/zram.rs`

**Configuration**:
- Pool size: 512 KiB (128 frames × 4 KiB)
- Metadata slots: 128 entries
- Compression: LZ4 (via `lz4_flex` crate)
- Allocation: Fixed pre-allocated, no runtime heap
- Initialization: Called at boot via `zram::init()`

### ZRAM API

```rust
// Initialize at boot
pub fn init() { /* allocates static pool */ }

// Compress and store a page
pub fn write_page(page_index: usize, raw_data: &[u8; 4096]) 
    -> Result<usize, ZramError>

// Decompress and retrieve a page
pub fn read_page(page_index: usize, out_buffer: &mut [u8; 4096]) 
    -> Result<(), ZramError>

// Query compression statistics
pub fn stats() -> (usize, usize)  // (total_compressed, pages_stored)
```

### Integration Points (Phase 5 Plan)

**1. Memory Pressure Trigger** (TBD)
```rust
// In kernel memory pressure handler (when PMM free < threshold)
if free_frames < PRESSURE_THRESHOLD {
    // Identify least-recently-used pages
    // Compress to ZRAM via write_page()
    // Free the original frame
}
```

**2. Page Fault Handler** (TBD)
```rust
// When process accesses missing page
if page_in_zram(page_index) {
    // Decompress from ZRAM via read_page()
    // Allocate new frame
    // Copy decompressed data
    // Map frame to process
}
```

**3. Statistics & Monitoring**
```rust
// Expose ZRAM stats via /proc/zram or similar
pub fn get_compression_ratio() -> (usize, usize) {
    let (compressed, count) = zram::stats();
    (compressed, count * 4096)  // (used, original)
}
```

---

## SunlightFS Block Device Interface (Phase 5 Plan)

### Block Device Abstraction

**Planned Implementation**:

```rust
pub trait BlockDevice {
    fn read_blocks(&mut self, lba: u64, count: usize, buf: &mut [u8; 4096]) 
        -> Result<(), BlockError>;
    
    fn write_blocks(&mut self, lba: u64, count: usize, data: &[u8; 4096]) 
        -> Result<(), BlockError>;
}

pub struct SunlightFS {
    device: Box<dyn BlockDevice>,
    block_cache: BlockCache,
    superblock: Superblock,
}
```

### Boot-time Mount Flow

```
1. VFS Server starts
2. Parse /etc/fstab:
   /dev/sda1   /boot    bootfs   defaults
   /dev/sda2   /        sunlightfs   defaults

3. For /dev/sda2 with sunlightfs:
   a. Open block device (/dev/sda2)
   b. Read superblock
   c. Validate SunlightFS signature
   d. Mount as "/" in VFS
   
4. VFS now routes "/" requests to SunlightFS block device
```

---

## Current Scheduler Status

### Round-Robin Scheduling (Reverted)

**Reason for revert**: BORE scheduler's multi-tier queue introduced IPC latency freezes on microkernel paths.

**Current behavior**:
- Simple round-robin: each process gets equal timeslice
- No priority tiers
- No interactive vs. CPU-bound detection
- Highly predictable timing (good for microkernel IPC)

**Trade-off**:
- ✓ Stable, predictable IPC latency
- ✓ No complex state machine
- ✗ No interactive/background distinction
- ✗ Less responsive UI

---

## FSTAB Format & Parsing

### Current /etc/fstab Seed File

**Location**: `sunlight-fs/etc/fstab` (seeded in INITRAMFS)

**Format**:
```
device      mountpoint  fstype      options
/dev/sda1   /boot       bootfs      defaults
/dev/sda2   /           sunlightfs  defaults
/dev/ram0   /           ramfs       defaults      # Fallback
```

### Parsing Implementation

**Location**: `sunlight-fs/src/fstab.rs`

```rust
pub struct FstabEntry<'a> {
    pub device: &'a str,
    pub mountpoint: &'a str,
    pub fs_type: &'a str,
    pub options: &'a str,
}

pub fn parse_fstab(text: &str) -> FstabTable<'_> {
    // 1. Skip comments (#) and empty lines
    // 2. Split on whitespace
    // 3. Require all 4 fields
    // 4. Return up to MAX_FSTAB_ENTRIES (8)
}
```

**Safety**:
- ✓ No panics on malformed input
- ✓ Skips incomplete lines
- ✓ Ignores extra whitespace
- ✓ Comments stripped cleanly

---

## VFS Mount Coordination

### Current Mount Flow (VFS Server)

**Location**: `services/vfs_server/src/main.rs:207-247`

```
Step 1: Create seed VFS
        Mount INITRAMFS at /

Step 2: Read /etc/fstab from seed VFS

Step 3: Parse FSTAB entries

Step 4: For each entry:
        Call mount_fstab_entry()
        
Step 5: mount_fstab_entry() dispatcher:
        case "ramfs" → mount_ramfs()
        case "bootfs" → mount BootFS (FAT32)
        case "sunlightfs" → (TBD)
        
Step 6: Route requests based on mount table
        "/" → VFS handler
        "/boot" → BootFS handler
        "/dev/..." → Block device handler
```

### Handle Encoding

```
Handle format: [mount_idx (8 bits) | local_handle (24 bits)]

mount_idx:
  0 = RAMFS
  1 = BootFS (FAT32)
  2+ = Block devices (SunlightFS, etc.)

VFS routing:
  unpack_handle(raw_handle) → (mount_idx, local_handle)
  switch mount_idx {
      0 → ramfs.read(local_handle, ...)
      1 → boot.read(local_handle, ...)
      2+ → block_device.read(local_handle, ...)
  }
```

---

## Integration Checklist for Phase 5

### Currently Implemented ✅

- [x] ZRAM memory compression (512 KiB pool, LZ4)
- [x] FSTAB parsing (handles comments, whitespace, missing fields)
- [x] VFS mount coordinator (routing to RAMFS/BootFS)
- [x] Handle encoding scheme (mount + local handle)
- [x] Boot filesystem (FAT32 read-only)
- [x] RAMFS filesystem (INITRAMFS read-write)

### To Implement (Phase 5.1) 🔄

- [ ] SunlightFS block device filesystem
  - [ ] Superblock format design
  - [ ] Inode structure and allocation
  - [ ] Data block allocation
  - [ ] Directory entry format
  - [ ] Journal/crash recovery
  
- [ ] Block device abstraction
  - [ ] VirtIO block device driver
  - [ ] ATA/IDE support (optional)
  - [ ] SD card support (optional)

- [ ] Memory pressure integration
  - [ ] Monitor PMM free frame count
  - [ ] Trigger page compression when pressure > threshold
  - [ ] Implement LRU page selection
  - [ ] Handle page decompression on fault

- [ ] Persistent storage features
  - [ ] Write-back caching
  - [ ] Journal commit
  - [ ] Fsck/recovery tools

### Phase 6+ Enhancements 🚀

- [ ] Advanced filesystems (Btrfs, ext4)
- [ ] NVM/NVMe support
- [ ] RAID/redundancy
- [ ] Encryption (LUKS/dm-crypt)
- [ ] Distributed filesystem (NFS, Ceph)
- [ ] Tiered storage (SSD + HDD)

---

## Memory Optimization Strategy

### Current Footprint

```
Component           Size        Purpose
──────────────────────────────────────────
Kernel (core)       ~512 KiB    Boot + essentials
Heap                1 MiB       Allocations
Stack (kernel)      32 KiB      Each process
ZRAM pool           512 KiB     Compressed pages
VMM tables          ~256 KiB    Page table cache

INITRAMFS           16 KiB      Boot filesystem
FAT32 boot          512 MB      /boot partition
Root (SunlightFS)   ~5 GB       / partition

Total boot:         ~2 MiB      (in memory)
```

### Compression Efficiency

**Expected compression ratios** (with LZ4):
- Zero pages: 99% (64 bytes from 4096)
- Text files: 80-90% (400-800 bytes from 4096)
- Code: 70-85% (600-1200 bytes from 4096)
- Incompressible: 100% (rejected by ZRAM)

**Example**: 512 MiB uncompressed pages
- Worst case: 512 MiB (no compression)
- Typical case: 50-100 MiB (with compression)
- Best case: 5-10 MiB (highly repetitive)

---

## Performance Characteristics

### Latency Profile

```
Operation               Latency      Notes
─────────────────────────────────────────────
Round-robin reschedule  ~1 μs        (no BORE overhead)
IPC call               ~10 μs        (microkernel path)
Page fault handler     ~50 μs        (without compression)
ZRAM write             ~100 μs       (LZ4 + store)
ZRAM read              ~50 μs        (decompress only)
Block device read      ~1-10 ms      (seek + transfer)
Cache hit              ~1 μs         (memory access)
```

### Throughput

```
Operation               Throughput
─────────────────────────────
Memory access          ~10 GB/s
Compression (LZ4)      ~500 MB/s
Block device           ~100 MB/s (QEMU virtio)
Network (Ethernet)     ~100 MB/s (simulated)
```

---

## Testing Plan

### Unit Tests

- [x] FSTAB parsing (with/without comments, whitespace, malformed)
- [x] ZRAM compression/decompression round-trip
- [x] Handle encoding/decoding
- [ ] SunlightFS superblock validation
- [ ] Block device mock tests

### Integration Tests

- [ ] Boot with multiple mount entries
- [ ] Survive /boot missing (fallback to RAMFS)
- [ ] Read files from RAMFS, BootFS, SunlightFS
- [ ] Write to RAMFS and verify persistence
- [ ] Compress pages under memory pressure
- [ ] Decompress on demand

### Stress Tests

- [ ] 1000 concurrent file opens
- [ ] Rapid mount/unmount cycles
- [ ] Full ZRAM pool saturation
- [ ] Large file transfers (>100 MB)
- [ ] Network I/O during memory pressure

---

## Debugging Tools (Future)

```bash
# Inspect ZRAM statistics
cat /proc/zram/stats

# Mount table query
mount -l

# Block device info
lsblk -a

# Memory pressure monitoring
watch -n 1 'cat /proc/meminfo'

# Filesystem integrity check
sunlightfs-fsck /dev/sda2
```

---

## Conclusion

**Phase 5 Status**: Memory compression (ZRAM) and filesystem coordination (FSTAB) are architecturally sound and partially implemented. The VFS multiplexer cleanly separates boot/RAM/block filesystems. The missing piece is the SunlightFS implementation for persistent block storage.

**Next steps**:
1. Design SunlightFS superblock/inode format
2. Implement block device abstraction
3. Integrate memory pressure handling with ZRAM
4. Test multi-filesystem boot sequence

**Phase 5 Ready**: ✅ ZRAM, ✅ FSTAB, ⏳ SunlightFS, ⏳ Memory pressure triggers
