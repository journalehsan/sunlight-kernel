# Phase 6 Quick Start Guide

## Overview

SunlightOS Phase 6 transforms the live-boot environment into a **persistent, self-hosting OS** through automated disk provisioning and an interactive installation wizard.

---

## Installation in 3 Steps

### Step 1: Create Disk Image (Host Machine)

```bash
$ cd /path/to/sunlightos-kernel
$ ./tools/run.sh --disk-only

[QEMU] Creating persistent virtual disk...
[QEMU] Creating 5GB raw disk image...
[QEMU] ✓ Disk created: target/sunlightos-disk.img
```

**Output**: `target/sunlightos-disk.img` (5GB raw disk)

---

### Step 2: Boot Live SunlightOS with Disk (Host Machine)

```bash
$ ./tools/run.sh --build --disk

[Build] Rebuilding kernel and services...
[QEMU] Creating persistent virtual disk...
[QEMU] ✓ Disk exists: target/sunlightos-disk.img
[QEMU] Starting QEMU...
[Boot] SunlightOS Phase 3 OK
[Init] login:
```

**Inside QEMU VM**: SunlightOS is running from live ISO, and `/dev/sda` (5GB) is available for installation.

---

### Step 3: Run Interactive Installer (Inside VM)

```bash
sunlight-vm$ sudo ./tools/install_sunlightos.sh /dev/sda

╔════════════════════════════════════════════════════╗
║     SunlightOS Interactive System Installer        ║
║                  Phase 6 - Release                 ║
╚════════════════════════════════════════════════════╝

[INFO] Checking System Requirements...
[INFO] ✓ All requirements met

[INFO] Verifying Target Disk...
[INFO] Target disk: /dev/sda (5GB)

⚠ WARNING: This will erase all data on /dev/sda
Type 'yes' to continue: yes

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
System Configuration
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

Hostname Configuration
Enter system hostname: myhost

Timezone Configuration
Enter timezone (e.g., Asia/Tehran, UTC): UTC

Root Administrative Account
Set root password: (hidden input)
Confirm password: (hidden input)

Standard User Account
Enter standard user username: alice
Set password for alice: (hidden input)
Confirm password: (hidden input)

[INFO] Configuration complete:
  Hostname:  myhost
  Timezone:  UTC
  Root:      (configured)
  User:      alice (configured)

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
Partitioning Disk
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

[INFO] Clearing partition table...
[INFO] Creating partitions...
[INFO] ✓ Partitions ready

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
Formatting Partitions
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

[INFO] Formatting /dev/sda1 as FAT32...
[INFO] Formatting /dev/sda2 as ext4...

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
Installing Limine Bootloader
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

[INFO] Installing Limine to MBR of /dev/sda...

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
Cloning Live Filesystem
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

[INFO] Cloning root filesystem to persistent storage...
[INFO] ✓ Filesystem cloned

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
Generating Configuration Files
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

[INFO] Writing hostname...
[INFO] Writing timezone configuration...
[INFO] Generating user accounts...
[INFO] Generating fstab...

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
Installation Complete ✓
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

[INFO] ✓ SunlightOS successfully installed to /dev/sda

System Details:
  Hostname:       myhost
  Timezone:       UTC
  Root partition: /dev/sda2 (ext4)
  Boot partition: /dev/sda1 (FAT32)

Next Steps:
  1. Power down the system
  2. Attach the disk permanently to the VM or physical system
  3. Boot into your installed SunlightOS
```

---

## What Gets Installed

### Disk Layout

```
/dev/sda
├── /dev/sda1 (512 MB FAT32)    → /boot
│   ├── boot/sunlight-kernel.elf
│   ├── boot/limine/limine.conf
│   └── boot/limine/limine-bios.sys
│
└── /dev/sda2 (~4.5 GB ext4)   → /
    ├── bin/, lib/, etc/
    ├── /etc/hostname           (system name)
    ├── /etc/localtime          (timezone)
    ├── /etc/auth/users.json    (user accounts)
    └── /etc/fstab              (partition mounts)
```

### Configuration Files Generated

**1. `/etc/hostname`** — System hostname
```
myhost
```

**2. `/etc/localtime`** — Timezone symlink/copy
```
→ /usr/share/zoneinfo/UTC
```

**3. `/etc/auth/users.json`** — User accounts (SHA256-hashed passwords)
```json
{
  "version": 1,
  "users": [
    {
      "uid": 0,
      "username": "root",
      "password_hash": "abc123...",
      "home": "/root"
    },
    {
      "uid": 1000,
      "username": "alice",
      "password_hash": "def456...",
      "home": "/home/alice"
    }
  ],
  "groups": [...]
}
```

**4. `/etc/fstab`** — Partition mount configuration
```
UUID=abc-123    /       ext4    defaults   0  1
UUID=def-456    /boot   vfat    defaults   0  2
/proc           /proc   proc    defaults   0  0
/sys            /sys    sysfs   defaults   0  0
```

---

## Usage Patterns

### Basic Installation (Most Common)

```bash
# 1. Create disk (one-time)
./tools/run.sh --disk-only

# 2. Boot and install (one-time)
./tools/run.sh --build --disk

# 3. Inside VM, run installer
sudo ./tools/install_sunlightos.sh /dev/sda

# 4. Shutdown VM
sudo poweroff
```

### Rebuild and Test

```bash
# Rebuild kernel, boot with existing disk
./tools/run.sh --build --disk

# Inside VM: Reinstall (wipes disk)
sudo ./tools/install_sunlightos.sh /dev/sda
```

### Boot Installed Disk (After Installation)

```bash
# Boot from disk directly (no ISO)
qemu-system-x86_64 \
  -drive file=target/sunlightos-disk.img,format=raw \
  -m 256M -vga std -serial stdio
```

### Create Fresh Disk

```bash
# Remove old image and create new one
rm target/sunlightos-disk.img
./tools/run.sh --disk-only
./tools/run.sh --disk
```

---

## Input Validation Rules

| Field | Rules | Examples |
|-------|-------|----------|
| **Hostname** | RFC 952 (a-z, 0-9, hyphen, 1-63 chars) | `myhost`, `web-01` ✓<br>`MyHost`, `_invalid` ✗ |
| **Timezone** | Must exist in `/usr/share/zoneinfo/` | `UTC`, `Asia/Tehran` ✓<br>`USA/Eastern`, `GMT+5` ✗ |
| **Username** | POSIX (a-z_, alphanumeric, 1-32 chars) | `alice`, `user_123` ✓<br>`123user`, `root` ✗ |
| **Password** | Any non-empty string, must confirm | `MyS3cur3P@ss!` ✓<br>`""` (empty) ✗ |

---

## Troubleshooting

### Issue: Disk not visible in VM

```bash
# Check inside VM
$ lsblk
NAME   MAJ:MIN RM SIZE RO TYPE MOUNTPOINT
sda      8:0    0   5G  0 disk
```

If not visible: Restart QEMU with `--disk` flag:
```bash
./tools/run.sh --disk
```

### Issue: Partitions not ready after formatting

The installer retries up to 5 times with 1-second delays. If still fails:

```bash
# Manually fix inside VM (if interrupted)
$ sudo partprobe /dev/sda
$ lsblk  # Verify partitions appear
```

### Issue: Installer stops during filesystem clone

Ensure sufficient disk space:

```bash
# Check in VM
$ df -h
Filesystem      Size  Used Avail Use%
tmpfs           256M  100M  156M  40%
/               256M  100M  156M  40%
```

If root (/) is >70% full, the clone may fail. Run from a fresh boot.

### Issue: Invalid hostname/timezone rejected

Check input format:

```bash
# Valid hostname
$ echo "myhost" | grep -E '^[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?$'
myhost

# Valid timezone
$ ls /usr/share/zoneinfo/UTC
/usr/share/zoneinfo/UTC
```

---

## Advanced Options

### Custom Target Disk

```bash
# Specify different disk (e.g., /dev/sdb)
sudo ./tools/install_sunlightos.sh /dev/sdb
```

### Network Block Device (future)

```bash
# For remote installation scenarios
sudo ./tools/install_sunlightos.sh nbd0
```

### Preserve Existing Data (future)

```bash
# Install to partition without erasing disk
./tools/install_sunlightos.sh --preserve /dev/sda2
```

---

## Performance Notes

| Operation | Time |
|-----------|------|
| Disk creation (`qemu-img create`) | ~1-2 sec |
| Partitioning | ~2 sec |
| Formatting (FAT32 + ext4) | ~5 sec |
| Filesystem clone (rsync) | ~30-60 sec (depends on disk I/O) |
| Configuration generation | <1 sec |
| **Total installation time** | **~40-70 seconds** |

---

## What's Installed vs. Live

| Component | Live (ISO) | Installed (Disk) |
|-----------|-----------|------------------|
| Kernel | Yes | Yes |
| Services | Yes (RAMFS) | Yes (ext4) |
| User accounts | Hardcoded | Generated |
| Timezone | System | Configured |
| Bootloader | Limine (CD boot) | Limine (MBR) |
| Root filesystem | RAMFS (2-3 GB limit) | ext4 (persistent) |

---

## After Installation

### Boot Installed System

```bash
# From host machine
qemu-system-x86_64 \
  -drive file=target/sunlightos-disk.img,format=raw \
  -m 256M -vga std -serial stdio

# From SunlightOS (persistent)
login: alice
password: (your password)

sunlight-vm$ whoami
alice

sunlight-vm$ cat /etc/hostname
myhost

sunlight-vm$ date
Mon Jun 12 00:00:00 UTC 2026
```

### System Administration

```bash
# Change hostname (persistent)
$ sudo echo "newhost" > /etc/hostname

# Switch timezones (persistent)
$ sudo cp /usr/share/zoneinfo/Asia/Tehran /etc/localtime

# Add user (updates /etc/auth/users.json)
$ sudo useradd bob

# Verify fstab
$ cat /etc/fstab
```

---

## Security Considerations

### Password Storage

- Passwords are **SHA256-hashed** (not salted, not bcrypt)
- Stored in `/etc/auth/users.json` (requires root read)
- **Not** suitable for production security (upgrade path for future phases)

### Filesystem Permissions

- Root partition cloned as-is from live environment
- No automatic permission hardening
- Recommend manual security review post-installation

### Bootloader

- Limine installed to MBR (no password protection)
- Future phase: UEFI Secure Boot support

---

## Phase 6 Checklist

- [x] Enhanced `run.sh` with `--disk` and `--disk-only` flags
- [x] Interactive `install_sunlightos.sh` wizard (420 lines)
- [x] Input validation (hostname, timezone, username, password)
- [x] Disk partitioning (512MB FAT32 + 4.5GB ext4)
- [x] Limine bootloader installation to MBR
- [x] Live filesystem cloning via rsync
- [x] Configuration file generation (hostname, localtime, users.json, fstab)
- [x] Error handling and safety checks
- [x] Comprehensive documentation
- [x] Quick start guide

**Status: ✅ Phase 6 Complete — SunlightOS is now a persistent, self-hosting OS**

---

## Next Steps (Phase 7+)

1. **Graphical Installer** — GTK-based UI
2. **Network Configuration** — DHCP/static setup
3. **Package Manager** — Install/update software
4. **System Updates** — Secure boot, cryptographic verification
5. **Cloud Integration** — VM cloud-init support

---

For detailed technical documentation, see: `PHASE6_IMPLEMENTATION.md`
