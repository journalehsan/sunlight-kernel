# Repository Guidelines

## Project Structure & Module Organization

SunlightOS is a Rust workspace for a small OS kernel and supporting crates.
The root `Cargo.toml` lists four members: `kernel/`, `ipc/`, `drivers/`, and
`compat-linux/`. Kernel code lives in `kernel/src/`, with architecture-specific
x86_64 code under `kernel/src/arch/x86_64/`, memory management under
`kernel/src/memory/`, and scheduler code under `kernel/src/sched/`. Shared IPC
types belong in `ipc/src/`; driver framework code belongs in `drivers/src/`;
Linux compatibility work belongs in `compat-linux/src/`. Build and boot helper
scripts are in `tools/`, and bootloader configuration is in `limine.cfg`.

## Build, Test, and Development Commands

- `cargo build --package sunlight-kernel`: builds the kernel ELF using the
  configured `x86_64-unknown-none` target and linker script.
- `cargo check --workspace`: quickly type-checks all workspace crates.
- `./tools/build.sh`: builds the kernel, prepares a Limine ISO at
  `target/sunlightos.iso`, then launches QEMU with serial output.
- `./tools/test.sh`: runs the automated QEMU boot gate and verifies expected
  serial messages such as `[PMM]`, `[VMM]`, and `Phase 1 OK`.
- `./tools/disk.sh`: creates `target/sunlightos_disk.img`; this uses loop
  devices and requires `sudo`.

## Coding Style & Naming Conventions

Use Rust 2021 style with `rustfmt` defaults: four-space indentation, snake_case
modules/functions, CamelCase types, and SCREAMING_SNAKE_CASE constants. Keep
kernel-facing crates `#![no_std]` unless there is a deliberate architecture
reason to change that. Prefer small modules that match subsystem boundaries
already present in `kernel/src/`. The kernel denies warnings, so remove unused
items or gate unfinished code deliberately.

## Testing Guidelines

The primary integration test is the QEMU boot gate in `./tools/test.sh`. Update
its `EXPECTED` messages when intentionally changing boot output. For pure Rust
logic in non-kernel crates, add normal `#[cfg(test)]` unit tests near the code
and run them with `cargo test --workspace` when the target supports it. Keep
boot tests deterministic and serial-output based.

## Commit & Pull Request Guidelines

Recent history uses Conventional Commit-style subjects, for example
`feat: Phase 1 interrupts ...` and scoped prefixes such as `kernel:` or `init:`.
Use short imperative subjects and include the subsystem when helpful. Pull
requests should describe the changed subsystem, note boot/test results, mention
new dependencies or privileged tooling, and link related issues. Include serial
output snippets for kernel boot behavior changes.

## Security & Configuration Tips

Do not commit generated artifacts under `target/`, downloaded Limine sources, or
local disk images. Keep privileged operations inside scripts explicit, especially
loop-device, mount, and `sudo` usage. Toolchain and linker behavior are defined
by `rust-toolchain.toml` and `.cargo/config.toml`; update both with care.
