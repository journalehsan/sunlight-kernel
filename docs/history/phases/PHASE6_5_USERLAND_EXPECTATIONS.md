# Phase 6.5 — Userland Foundations: Expectations & Test Checklist

> Companion to [PHASE6_5_USERLAND_PLAN.md](PHASE6_5_USERLAND_PLAN.md).
> Check a box only after its test passes. A step is done when **all** its boxes
> are checked AND `./tools/test.sh` (existing gates) still passes — no regressions.

**Testing conventions used below**

- Boot headless: `./tools/run.sh -n` (serial on stdio, 2048 MiB RAM, virtio disk + NAT attached by default).
- For automated gates, emit a serial marker (`[RTC] OK`, `[ENV] OK`, …) at init time
  and add a `tools/tests/phase6_5_<n>.expected` file so `./tools/test.sh` can match it,
  same pattern as the existing phase gates.
- Interactive checks: boot `./tools/run.sh` (GTK) and type commands in sunshell.

---

## Step 1: Dynamic TTY & Sysfetch — ✅ DONE 2026-06-12 (gate: `./tools/test.sh phase6.5.1`)

### 1.1 RTC clock in TTY title bar
- [x] Kernel reads date/time from CMOS RTC (with update-in-progress guard and BCD/binary mode handling) — `kernel/src/arch/x86_64/rtc.rs`, reads CMOS once at boot then advances via PIT ticks
- [x] Title bar shows live clock in format `12:22 AM | 2026/6/12` — verified by screenshot (`5:51 AM | 2026/6/12`, matched host UTC)
- [x] Clock updates as time passes — refresh is event-driven (any keypress/output wake re-checks minute rollover); idle redraw needs a periodic timer notification, deferred until timer pub/sub exists

**Test**
```sh
# Marker gate: kernel logs the RTC read at boot
timeout 30 ./tools/run.sh -n 2>&1 | grep -E '\[RTC\].*20[0-9]{2}'
# Visual: boot GTK, confirm title bar text matches host `date` (QEMU passes host RTC by default)
./tools/run.sh
# Format check: time must match regex  ^(1[0-2]|[1-9]):[0-5][0-9] (AM|PM) \| 20[0-9]{2}/[0-9]{1,2}/[0-9]{1,2}$
```

### 1.2 Remove hardcoded RAM from sysfetch
- [x] No `256` MB literal (or any hardcoded total) remains in sysfetch code — grep clean; sysfetch/free/uptime/stats-header all use the SysInfo syscall

**Test**
```sh
grep -rn '256' sunshell/ | grep -i 'mem\|ram'   # must return nothing
```

### 1.3 Sysfetch reports real PMM / CPU / uptime values
- [x] New syscall exposes PMM total/used — syscall 82 `SysInfo` (4×u64: total_kb, used_kb, uptime_secs, unix_time); syscall 81 `GetTimeUtc`. NOTE: fixed `GetTimeUtc = 50` in sunlight-ipc, which collided with `sys_mmap` in the kernel dispatcher
- [x] `sysfetch` total RAM matches QEMU `-m` — verified 2038MB @ `-m 2048` and 1014MB @ `-m 1024` (screenshots)
- [x] Used RAM is non-zero and live — 24MB from real PMM frame counts
- [x] CPU model string read from CPUID (`QEMU Virtual CPU version 2.5+`), uptime from PIT ticks (33s at test time)

**Test**
```sh
# Boot twice with different RAM; sysfetch must report different totals
./tools/run.sh -n -m 1024   # type sysfetch via serial -> expect ~1024MB
./tools/run.sh -n -m 2048   # type sysfetch via serial -> expect ~2048MB
# Cross-check against the PMM boot line, e.g. "[PMM] 2023/2038 MiB free"
```

---

## Step 2: Environment Variables & Process State

### 2.1 Env var manager on the PCB
- [ ] `EnvMap` struct (key→value) attached to the process control block
- [ ] Defaults populated at process spawn: at least `PATH=/bin:/usr/bin`, `USER`, `HOME`
- [ ] Shell builtins work: `env` (list), `export KEY=VAL` (set), `echo $KEY` (expand)

**Test**
```sh
# In sunshell:
env                    # shows PATH and USER
export FOO=bar
echo $FOO              # prints: bar
```

### 2.2 PATH resolution
- [ ] Bare command (`ls`) is resolved by splitting `PATH` on `:` and probing each dir in the VFS
- [ ] First match wins (ordering respected); absolute paths (`/bin/ls`) bypass PATH search
- [ ] Miss produces a clean `command not found: <name>` (no panic)

**Test**
```sh
# In sunshell:
ls                          # resolves via PATH and runs
/bin/ls                     # absolute path also runs
export PATH=/nonexistent
ls                          # prints "command not found: ls", shell survives
```

---

## Step 3: VFS & Binary Loading — ⏳ Parts A & B done 2026-06-12, see [PHASE6_5_STEP3_VFS_ELF.md](PHASE6_5_STEP3_VFS_ELF.md)

### 3.1 Luxos reference analysis
- [x] Notes written (docs/ or code comments) on lxfs on-disk layout and lucerna's exec/loader path, with what we adopt vs. reject for Rust — [PHASE6_5_STEP3_VFS_ELF.md](PHASE6_5_STEP3_VFS_ELF.md)

### 3.2 VFS mounting: FAT32 + SunlightFS
- [x] `mount` mechanism in sunlight-fs with a mount table routing paths by longest-prefix — `Vfs::mount`/`mount_fat`; `umount` (+ cache flush) still open
- [x] FAT32 read support (dir listing + file read) against a real image — unit-tested against synthetic images (`sunlight-fat::testimg`); in-VM check against a `mkfs.vfat` image still open
- [ ] SunlightFS partitions mountable through the same interface

**Test**
```sh
# Prepare a FAT32 test image on the run.sh disk (host side):
qemu-img create -f raw target/fat32_test.img 64M
mkfs.vfat target/fat32_test.img && mcopy -i target/fat32_test.img README.md ::/hello.txt
./tools/run.sh -n --disk target/fat32_test.img
# In sunshell:
mount /dev/vda /mnt fat32
ls /mnt                # shows hello.txt
cat /mnt/hello.txt     # prints file contents
```

### 3.3 Kernel ELF loader executes sunlight-utils
- [x] ELF64 header + program-header parsing with validation (magic, machine, PT_LOAD bounds) — `sunlight_elf::plan_segments`, kernel loader rewritten on top of it
- [x] Segments mapped at user VAs with correct R/W/X permissions, BSS zeroed — incl. W^X enforcement, shared-page flag union, filesz-bounded copy
- [ ] `ls`, `mkdir`, `ping` from sunlight-utils launch via PATH, run, and return an exit code — blocked on routing `sys_exec` through the VFS (still resolves embedded paths only)
- [x] Corrupt/non-ELF file is rejected with an error, not a kernel panic — `load_elf` returns None with a serial log on any validation failure; covered by 9 rejection unit tests

**Test**
```sh
# In sunshell:
ls /                   # external binary executes and prints listing
mkdir /tmp/x && ls /tmp   # mkdir took effect
cat /etc/passwd; echo $?   # exit code 0 propagated
exec /etc/passwd       # not an ELF -> clean error, no panic
```

---

## Step 4: Swap & Virtio

### 4.1 virtio-blk initialization
- [ ] sunlight-virtio initializes the block device (legacy mode, matches `disable-modern=on` used by run.sh/test.sh)
- [ ] Read AND write of sectors verified (write survives a guest reboot)

**Test**
```sh
timeout 30 ./tools/run.sh -n 2>&1 | grep '\[VIRTIO-BLK\].*OK'
# Persistence: write a file to the mounted disk, reboot, file still present
```

### 4.2 sunlightd zram swap
- [ ] sunlightd allocates a compressed zram swap block over the virtio interface at boot
- [ ] `free` in sunshell shows non-zero swap total
- [ ] Memory-pressure test: allocate past physical RAM, swap-used rises, no OOM panic

**Test**
```sh
timeout 30 ./tools/run.sh -n 2>&1 | grep -E '\[ZRAM\].*(OK|enabled)'
# In sunshell:
free                   # Swap total > 0
# Stress with low RAM so swap engages:
./tools/run.sh -n -m 512    # run memory hog util; verify swap-used grows in `free`
```

---

## Final Gate (all steps)

- [ ] `./tools/test.sh` — all pre-existing phase gates still pass
- [ ] `./tools/run.sh --build` — clean rebuild, boots to shell with clock, env, exec, and swap all live
