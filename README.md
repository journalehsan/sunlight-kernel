# SunlightOS

**SunlightOS** is an independent operating system written in Rust, featuring a custom capability-based async IPC microkernel with Linux binary compatibility.

## Architecture

- **Microkernel (Ring 0):** Contains only the scheduler, IPC bus, memory manager, and capability broker.
- **User-space (Ring 3):** All drivers, filesystems, and services run here.
- **Helios Subsystem:** Provides Linux binary compatibility by translating Linux syscalls to IPC messages.

## Prerequisites

- **Rust** (nightly toolchain — managed by `rust-toolchain.toml`)
- **QEMU** (`qemu-system-x86_64`)
- **xorriso** (for ISO creation)
- **git**, **make**, **gcc** (for building Limine bootloader)
- **parted**, **dosfstools**, **kpartx** (optional, for `disk.sh`)

Install on Debian/Ubuntu:
```bash
sudo apt-get update
sudo apt-get install -y qemu-system-x86 xorriso git make gcc parted dosfstools
```

## Build Instructions

The first time, the toolchain will be automatically installed by rustup when you run cargo commands.

### Build and run (interactive):
```bash
./tools/build.sh
```
This compiles the kernel, creates a bootable ISO with the Limine bootloader, and launches QEMU with serial output.

### Run automated test:
```bash
./tools/test.sh
```
This builds (if needed), runs QEMU with a timeout, and asserts that the expected boot messages are printed. Exits with code 0 on success, 1 on failure.

### Create test disk image:
```bash
./tools/disk.sh
```
Creates a 64MB FAT32 disk image at `target/sunlightos_disk.img`.

## Workspace Structure

```
sunlightos/
├── kernel/          # sunlight-kernel — Ring 0 microkernel (no_std)
├── ipc/             # sunlight-ipc — IPC message types (no_std + std)
├── drivers/         # sunlight-drivers — user-space driver framework (std)
├── compat-linux/    # sunlight-compat-linux — Helios Linux compatibility (std)
├── docs/            # project documentation and phase summaries
└── tools/           # build scripts, test harness, disk tools
```

## Current Status

| Phase | Status |
|-------|--------|
| Phase 0: Toolchain & Environment | **In Progress** |
| Phase 1: Memory Management | Planned |
| Phase 2: IPC & Capabilities | Planned |
| Phase 3: Drivers | Planned |
| Phase 4: Linux Compatibility (Helios) | Planned |

## License

MIT / Apache-2.0 (to be determined)
