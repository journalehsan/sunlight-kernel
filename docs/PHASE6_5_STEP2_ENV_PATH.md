# Phase 6.5 Step 2 — Environment Variables & PATH Resolution

## What was built

### 1. Kernel: `EnvMap` on the Process Control Block

New file `kernel/src/process/env.rs`:

- `EnvMap` — `BTreeMap<String, String>`-backed key→value registry
  (no_std + alloc; ordered, deterministic `env` listings, no hasher needed
  in the kernel).
- `EnvMap::with_defaults(uid, username)` — seeds `PATH`, `USER`, `HOME`,
  `SHELL`. `DEFAULT_PATH = /bin:/usr/bin:/sunlight-utils:/sunlight-net-utils`.
- `EnvMap::inherit(&parent)` — fork/spawn inheritance.
- `EnvMap::to_envp()` — serializes to `KEY=VALUE` strings for SysV envp
  stack marshalling in `exec_into_process` (full stack write lands with the
  Step 3 ELF loader).

PCB changes (`kernel/src/process/`):

- `Process.env: EnvMap` field added (`mod.rs`); empty at `Process::new`.
- `spawn.rs`: `spawn_from_path` now sets `process.uid/gid` and populates
  `process.env` with defaults; new `spawn_from_path_with_env(..., env:
  Option<EnvMap>)` lets a caller pass an inherited/customized environment.
- `fork.rs`: both fork paths clone the parent environment via
  `EnvMap::inherit`.

### 2. Shell: `ShellEnv` + builtins (`sunshell`, no_std build)

New file `sunshell/src/shellenv.rs` mirrors the kernel defaults and adds
`expand_token` (`$KEY` / `${KEY}` → value, unset → empty string).

`sunshell/src/main.rs` (sunlight module):

- `Shell.env: ShellEnv`, seeded by `init_env()` in `_start` after the
  user identity is resolved from /etc/passwd via GETPWUID.
- `run_line` expands `$VAR` tokens in arguments before dispatch (so
  `echo $FOO` works for any builtin).
- New builtins: `env` (lists KEY=VALUE via the long-output path),
  `export KEY=VALUE`, `unset KEY`.
- Unknown commands no longer print a bare "command not found": they go
  through `resolve_in_path`, which splits `$PATH` on `:` and probes each
  `dir/cmd` candidate with a VFS `STAT` (first match wins; commands
  containing `/` bypass the search). Hits report the resolved path —
  actual execution arrives with the Step 3 ELF loader. Misses print
  `sshl: command not found: <name>` and the shell survives.

### 3. VFS: applet stubs for resolution targets

`sunlight-fs/src/ramfs.rs` INITRAMFS now seeds `/sunlight-utils/` (ls, cat,
cp, mv, rm, mkdir, …) and `/sunlight-net-utils/` (ping, ifconfig, wget,
curl, dig, …) with mode-755 stub files so PATH probing has real targets.
`RAMFS_MAX_ENTRIES` raised 64 → 128 to keep headroom for runtime file
creation.

## Constraints

- VFS IPC path encoding carries max 32 bytes (4 words); `stat_is_file`
  rejects longer candidate paths.
- Shell ↔ kernel env are separate copies by design: `export` mutates the
  shell's registry; propagating to spawned children goes through
  `spawn_from_path_with_env` once Step 3 wires exec.

## Verification status

- `cargo build -p sunlight-kernel` — OK
- `cargo build -p sunshell --features sunlight --no-default-features --release` — OK
- `cargo build -p sunlight-vfs-server --release` — OK
- `cargo test -p sunlight-fs` (host target) — 32/32 passing
- In-QEMU acceptance tests (`env`, `export FOO=bar; echo $FOO`, `ls`
  resolution, `export PATH=/nonexistent; ls` miss) — **pending a full
  image build + run** (`./tools/build.sh && ./tools/run.sh`).
