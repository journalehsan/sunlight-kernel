# Phase 6.5 — Userland Foundations Plan

> Saved 2026-06-12. Companion checklist: [PHASE6_5_USERLAND_EXPECTATIONS.md](PHASE6_5_USERLAND_EXPECTATIONS.md)

## Role & Context

Expert systems programming / OS architecture work on **SunlightOS**, a custom OS
written in Rust. The OS currently has a basic kernel, ACPI support, a TTY, and a
shell (`sunshell`), but needs foundational improvements to userland execution,
memory reporting, and VFS.

Reference C-based OS: **Luxos** at `~/Projects/luxos/`. Use its filesystem
(`luxos/servers/fs/lxfs/`), kernel (`luxos/kernel`), and libc (`luxos/lucerna`)
as architectural inspiration — but our implementation must be idiomatic, safe Rust.

## Relevant Repository Areas

- `sunlight-utils/` & `sunlight-net-utils/` — binaries like `ls`, `mkdir`, `ping` (currently failing to load)
- `sunlight-fs/` — our custom filesystem, needs improvement
- `sunshell/` — the shell / sysfetch UI
- `sunlight-virtio/` & `sunlightd/` — virtio drivers and init daemon

## Mission

Execute the following steps **sequentially**. Do NOT move to the next step until
the current one compiles and is verified (see the expectations file for the
verification procedure of each step).

### Step 1: Dynamic TTY & Sysfetch

1. Modify the TTY rendering loop to fetch the current date and time from the RTC
   (Real-Time Clock) via CMOS/ACPI. Display it in the TUI title bar in this
   format: `12:22 AM | 2026/6/12`.
2. Locate the sysfetch logic in `sunshell`. Remove the hardcoded 256MB RAM values.
3. Wire sysfetch to query the kernel's memory allocator/PMM to retrieve
   real-time total RAM (e.g., 2048MB) and used RAM, as well as dynamic CPU and
   Uptime metrics.

### Step 2: Environment Variables & Process State

1. Implement an Environment Variable manager: a Rust struct holding key-value
   pairs (e.g., `PATH`, `USER`) attached to the process control block.
2. Implement PATH resolution: when a user types `ls`, the shell must split the
   `PATH` variable by `:` and search those directories in the VFS for the binary.

### Step 3: VFS & Binary Loading (Referencing Luxos)

1. Read and analyze the C code in `~/Projects/luxos/servers/fs/lxfs/` and
   `~/Projects/luxos/lucerna`.
2. Upgrade `sunlight-fs` to support mounting mechanisms. Add support for
   mounting FAT32 and SunlightFS partitions.
3. Implement an ELF Loader in the kernel: read the binary path resolved in
   Step 2, parse the ELF headers, allocate memory segments, and execute the
   binaries located in `sunlight-utils`.

### Step 4: Swap & Virtio

1. Review `sunlight-virtio`. Implement virtio block device initialization.
2. Update `sunlightd` to act as a zram generator, allocating a compressed swap
   block using the virtio interface.

## Working Agreement

- Begin with Step 1: RTC time fetching + dynamic memory querying for `sunshell`.
- Wait for confirmation before proceeding to each next step.
- Each step's acceptance criteria and test commands live in
  [PHASE6_5_USERLAND_EXPECTATIONS.md](PHASE6_5_USERLAND_EXPECTATIONS.md) —
  check the boxes there as items are verified.
