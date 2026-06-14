# sunlightd Implementation Summary

## Status: Partial - Core implementation complete, boot integration needs debugging

## Date: 2026-06-14

## Architecture

SunlightOS implements a two-layer init system:

1. **Kernel (Ring 0)**: Directly spawns core services during boot:
   - PID 1: sunlight-init (nameserver)
   - PID 2: timer_server
   - PID 3: vfs_server
   - PID 4: tty_server
   - PID 5: net_server
   - PID 6: sunlightd (service supervisor)

2. **sunlightd (Ring 3, PID 6)**: User-space service supervisor that:
   - Loads .service and .socket unit files
   - Builds dependency graphs
   - Monitors service health
   - Restarts crashed services
   - Exposes IPC control interface

## Implemented Components

### Sub-phase B.1: Crate Scaffold ✓
- Created `services/sunlightd/` with proper module structure
- Created `services/sunlightctl/` client binary
- Added to workspace Cargo.toml
- Configured userspace linker script via `.cargo/config.toml`

### Sub-phase B.2: Unit File Parser ✓
- Implemented in `services/sunlightd/src/unit.rs`
- Parses .service files with [Unit], [Service], [Install] sections
- Parses .socket files with [Socket] section
- Uses heapless types for no_std compatibility
- Supports all specified keys from design document

### Sub-phase B.3: Dependency Graph ✓
- Implemented in `services/sunlightd/src/graph.rs`
- Kahn's algorithm for topological sort
- Fixed-capacity arrays (MAX_UNITS = 32)
- Detects circular dependencies

### Sub-phase B.4: Process Supervisor ✓
- Implemented in `services/sunlightd/src/supervisor.rs`
- ServiceState enum tracks lifecycle
- Restart policy enforcement (No, OnFailure, Always)
- Restart rate limiting (5 restarts in 30 seconds)

### Sub-phase B.5: IPC Control Interface ✓
- Implemented in `services/sunlightd/src/ipc.rs`
- SunlightdOp enum defines control opcodes:
  - Start, Stop, Restart, Reload (management)
  - Status, List (query)
  - GetLog (logging)
- Message packing/unpacking for 80-byte IpcMsg

### Sub-phase B.6: sunlightctl Client ✓
- Implemented in `services/sunlightctl/src/main.rs`
- Commands: start, stop, restart, status, list, reload
- Looks up sunlightd via nameserver
- Sends IPC messages and displays results

### Sub-phase B.7: Socket Activation (Stub)
- Implemented in `services/sunlightd/src/socket_act.rs`
- Parses .socket units with ListenStream
- TCP port support designed
- **Deferred**: Actual NetOp::SocketBind and Accept require net_server IPC extension

### Sub-phase B.8: Journal Logging (Stub)
- Implemented in `services/sunlightd/src/journal.rs`
- LogCapture struct for stdout/stderr buffering
- VFS write path for `/var/log/<unit>.log`
- **Deferred**: Requires pipe IPC (Phase pipes)

### Sub-phase B.9: Kernel Integration ✓
- Added SUNLIGHTD_ELF_BYTES embed to kernel/src/main.rs
- Spawns sunlightd as PID 6 after net_server
- Process::new and ELF loading successful
- **Issue**: sunlightd created but not yet printing output (scheduling investigation needed)

### Sub-phase B.10-B.12: Testing and Documentation
- **Partial**: Boot integration needs debugging
- Expected serial output defined in tools/tests/sunlightd.expected
- sunlightd loads and spawns successfully
- Scheduler creates process #6 but output not yet visible

## Supported Unit File Syntax

### [Unit] Section
- `Description=` - human-readable service name
- `After=` - start ordering dependencies
- `Requires=` - hard dependencies
- `Wants=` - soft dependencies

### [Service] Section
- `Type=` - simple, oneshot, notify (notify is stub)
- `ExecStart=` - main service binary path
- `ExecStartPre=` - pre-start command
- `ExecStop=` - stop command
- `Restart=` - no, on-failure, always
- `RestartSec=` - seconds between restart attempts (default 5)
- `Environment=` - KEY=VALUE environment variables
- `EnvironmentFile=` - path to env file
- `User=` - which user to run as (default root)
- `WorkingDirectory=` - initial working directory
- `StandardOutput=` - journal, null, inherit
- `StandardError=` - journal, null, inherit

### [Install] Section
- `WantedBy=` - which target activates this unit

### [Socket] Section
- `ListenStream=` - port number or Unix socket path
- `Service=` - which .service to activate

### Supported Targets
- `sunlight.target` - normal operation
- `network.target` - network-dependent services

## Deferred Features

### Requires Pipe IPC (Future Phase)
- Journal logging (stdout/stderr capture)
- Service log files in /var/log/
- Log rotation

### Requires Net Server Extensions
- Socket activation (AF_INET)
- Unix domain sockets (AF_UNIX)

### Not Yet Implemented
- ExecStartPre/ExecStop command execution
- EnvironmentFile parsing
- User switching (needs /etc/passwd integration)
- WorkingDirectory changes
- Type=notify (sd_notify protocol)
- Actual service spawning (currently services are pre-spawned by kernel)

## Known Issues

1. **sunlightd output not visible**: Process spawns successfully but doesn't print startup messages
   - Likely cause: Scheduling order or early boot timing
   - Services are already running when sunlightd starts
   - Investigation needed: scheduler round-robin behavior at boot

2. **Service lifecycle not active**: sunlightd currently marks pre-spawned services as "running"
   - Actual spawn-on-demand requires init handoff redesign
   - Death notification mechanism not yet connected

3. **Build system integration**: Requires explicit RUSTFLAGS for userspace linking
   - Workaround: `RUSTFLAGS="-C link-arg=-Tservices/user-space.ld -C relocation-model=static"`
   - Root .cargo/config.toml uses kernel linker script

## Compatibility Notes for Next Phase

Phase 6 (desktop environment) can rely on:
- Unit file format compatibility with systemd subset
- .service files in /etc/sunlight/services/
- IPC control interface via sunlightctl
- Dependency ordering via After/Requires/Wants

Phase 6 should NOT rely on:
- Automatic service restart (restart monitoring not yet active)
- Socket activation
- Journal logging (logs to VFS)

## Build Instructions

```bash
# Build sunlightd with correct linker script
RUSTFLAGS="-C link-arg=-Tservices/user-space.ld -C relocation-model=static" \
  cargo build --package sunlightd --release --target x86_64-unknown-none

# Build sunlightctl
RUSTFLAGS="-C link-arg=-Tservices/user-space.ld -C relocation-model=static" \
  cargo build --package sunlightctl --release --target x86_64-unknown-none

# Build kernel (embeds sunlightd)
cargo build --package sunlight-kernel --release

# Boot test
./tools/build.sh
```

## File Structure

```
services/sunlightd/
├── .cargo/
│   └── config.toml          # Userspace linker config
├── Cargo.toml
└── src/
    ├── main.rs              # Entry point, IPC loop
    ├── unit.rs              # .service/.socket parser
    ├── graph.rs             # Dependency graph + topo sort
    ├── supervisor.rs        # Process lifecycle
    ├── ipc.rs               # Control interface
    ├── socket_act.rs        # Socket activation (stub)
    └── journal.rs           # Log capture (stub)

services/sunlightctl/
├── .cargo/
│   └── config.toml
├── Cargo.toml
└── src/
    └── main.rs              # CLI client

tools/tests/
└── sunlightd.expected       # Boot serial output expectations
```

## Next Steps

1. **Debug scheduling issue**: Investigate why sunlightd doesn't print output
   - Add debug serial output to track scheduler behavior
   - Verify sunlightd entry point is reached
   - Check if process yields correctly

2. **Connect death notifications**: Link process exit events to supervisor
   - Kernel PROC_EXIT IPC or polling mechanism
   - Trigger restart logic on service crash

3. **Implement actual spawning**: Transition from kernel-spawned to sunlightd-spawned services
   - Requires init handoff redesign
   - sunlight-init should spawn sunlightd first
   - sunlightd spawns all other services

4. **Add pipe IPC**: Enable journal logging
   - Kernel pipe() syscall
   - FD passing to child processes
   - Async drain from pipe to VFS

5. **Testing**: Once output is visible, implement tools/test.sh gate
   - Verify expected serial messages
   - Test sunlightctl commands
   - Validate restart behavior

## Lines of Code

- sunlightd: ~550 lines
- sunlightctl: ~200 lines
- Total new code: ~750 lines
- Modified kernel code: ~40 lines (embed + spawn)
