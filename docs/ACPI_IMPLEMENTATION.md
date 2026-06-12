# SunlightOS ACPI & Power Management Implementation (Phase 5.11)

## Overview
This document describes the complete ACPI (Advanced Configuration and Power Interface) implementation for SunlightOS x86_64, following a microkernel architecture pattern. The implementation provides power management capabilities (shutdown/reboot) with minimal Ring 0 complexity, following design patterns from the Lux kernel.

## Architecture Philosophy

### Microkernel Design Principle
- **Ring 0 (Kernel/Arch Layer)**: Handles only physical memory mapping, RSDP discovery, basic ACPI table parsing (RSDT/XSDT, FADT), and hardware register I/O
- **Ring 3 (User Space)**: Complex AML interpreter and power event routing belong in user space as future enhancements

This keeps the kernel lean, maintainable, and secure from bytecode parsing vulnerabilities.

## Implementation Structure

### Files Added/Modified

#### 1. **kernel/src/arch/x86_64/acpi.rs** (NEW)
Complete ACPI table discovery and power management implementation.

**Key Components:**

##### Data Structures
- `ACPIRSDP`: Root System Description Pointer (both ACPI 1.0 and 2.0+ formats)
- `ACPITableHeader`: Standard header present in all ACPI tables
- `ACPIRSDT`/`ACPIXSDT`: Root/Extended table directories (32-bit and 64-bit)
- `ACPIFADT`: Fixed ACPI Description Table with power control registers
- `ACPIMADT`: Multiple APIC Description Table (for multiprocessor support)
- `ACPIState`: Global state protected by mutex, stores discovered power parameters

##### Initialization Pipeline

**Milestone 1: Physical Table Discovery & Mapping**
```rust
pub unsafe fn init(rsdp_phys: u64) -> Result<(), &'static str>
```
- Receives RSDP physical address from Limine bootloader
- Verifies RSDP signature and checksums (sum of all bytes = 0 mod 256)
- Discovers tables through XSDT (ACPI 2.0+) or RSDT (ACPI 1.0)
- Maps all discovered tables and verifies their checksums

**Milestone 2: Parse FADT for Shutdown Primitives**
```rust
fn parse_fadt() -> Result<(), &'static str>
```
- Locates Fixed ACPI Description Table
- Extracts:
  - `SMI_CMD` port for ACPI enable
  - `PM1a_CNT_BLK` / `PM1b_CNT_BLK` (Power Management 1 Control Register Blocks)
  - `ACPI_ENABLE` / `ACPI_DISABLE` command bytes
  - Reset register information (I/O or memory-mapped)

**Milestone 3: DSDT Parsing & SLP_TYPx Extraction**
```rust
fn parse_dsdt(dsdt_phys: u64) -> Result<(), &'static str>
```
- Extracts pointer to DSDT (Differentiated System Description Table)
- Searches AML bytecode for `_S5` object (Soft Off sleep state)
- Locates `SLP_TYPa` and `SLP_TYPb` register values
- Default fallback values for QEMU/KVM virtual targets

**Milestone 4: Reset Port Primitives (Reboot)**
```rust
pub fn reboot() -> !
```
- Checks FADT for hardware reset register support
- Writes `RESET_VALUE` to `RESET_REG` (I/O port or memory-mapped)
- Fallback: 8042 Keyboard Controller pulse (write `0xFE` to port `0x64`)

**Milestone 5: Shutdown using S5 Sleep State**
```rust
pub fn shutdown() -> !
```
- Constructs S5 sleep payload: `(SLP_TYPa << 10) | (1 << 13)`
  - bits [12:10]: Sleep Type A
  - bit 13: Sleep Enable flag
- Writes to `PM1a_CNT_BLK` and `PM1b_CNT_BLK` (if present)
- System transitions to S5 (soft off) state

#### 2. **kernel/src/arch/x86_64/mod.rs** (MODIFIED)
- Added `pub mod acpi` declaration

#### 3. **kernel/src/main.rs** (MODIFIED)
- Added Limine RSDP request: `static RSDP_REQ: limine::request::RsdpRequest`
- Added ACPI initialization in boot sequence (after VMM, before IDT)
- Integrated ACPI into splash screen boot flow

#### 4. **kernel/src/arch/x86_64/syscall.rs** (MODIFIED)
- Added `PowerCtl = 80` syscall for power management
- Implemented `sys_powerctl(command: u64)` handler:
  - `0`: Shutdown
  - `1`: Reboot

#### 5. **sunshell/src/main.rs** (MODIFIED)
- Added `cmd_shutdown()` and `cmd_reboot()` methods
- Implemented syscall wrappers using inline assembly
- Updated help text to include new commands
- Commands trigger PowerCtl syscall (80) with appropriate command codes

## Execution Flow

### Boot Sequence
```
[PMM] Initialize physical memory
  ↓
[VMM] Initialize virtual memory & paging
  ↓
[ACPI] Discover and initialize ACPI tables
  ├─ Locate RSDP from Limine
  ├─ Verify checksums
  ├─ Walk RSDT/XSDT table directory
  ├─ Parse FADT
  ├─ Extract PM1a/PM1b control registers
  ├─ Search DSDT for _S5 sleep types
  └─ Ready for shutdown/reboot commands
  ↓
[IDT] Initialize interrupt handlers
  ↓
[Heap] Initialize kernel heap
  ↓
[Services] Load and initialize user-space services
```

### Shell Command Execution
```
user@sunlightos:~$ shutdown
  ↓
[TTY] cmd: shutdown -> Broadcasting system shutdown loop...
  ↓
syscall(80, 0)  // PowerCtl syscall with command=0
  ↓
[Kernel] sys_powerctl(0)
  ↓
[ACPI] shutdown()
  ├─ Write SLP_TYPa to PM1a_CNT_BLK
  ├─ Write SLP_TYPb to PM1b_CNT_BLK (if present)
  └─ System powers down (enters S5 sleep state)
```

## Diagnostic Output

When booting with ACPI initialization enabled, you'll see:
```
[ACPI] RSDP structure located at physical address: 0x000F6BC0
[ACPI] Checksum verified completely. ACPI Revision: 2.0 (XSDT active)
[ACPI] Found table: FACP (Fixed ACPI Description Table) at 0x07FE1020
[ACPI] Found table: APIC (MADT Multi-Processor Mapping) at 0x07FE12A0
[ACPI] PM1a_CNT_BLK port assigned: 0x0604
[ACPI] PM1b_CNT_BLK port assigned: 0x0000
[ACPI] Enabling ACPI mode via SMI_CMD port 0x00B2... Done.
[ACPI] _S5 Sleep Types: a=0x00, b=0x00
[ACPI] Initialization complete. System is ACPI revision 2
```

## Key Implementation Details

### Memory Safety
- Uses `spin::Mutex<ACPIState>` for thread-safe access to global ACPI state
- All unsafe blocks are carefully scoped and documented
- Pointer dereferencing only after validity checks

### Checksum Verification
- All ACPI tables have checksums verified (sum of all bytes mod 256 = 0)
- Corrupt tables are logged but don't halt boot (some tables may be optional)

### Table Walking
- Supports both RSDT (32-bit, ACPI 1.0) and XSDT (64-bit, ACPI 2.0+)
- Automatically detects and uses appropriate table format
- Handles variable-length table entries

### Hardware I/O
- Uses inline assembly for I/O port operations (`out dx, eax`)
- Supports both I/O port and memory-mapped register writes
- Respects GenericAddressStructure format from FADT

### AML Bytecode Parsing (Simplified)
- Limited _S5 object extraction without full AML interpreter
- Pattern matching for predictable bytecode sequence
- Fallback to default values for unknown systems

## Comparison with Lux Kernel

### Similarities
- Minimal ACPI implementation focused on shutdown/reboot
- Limine bootloader integration for RSDP discovery
- Table walking and checksum verification
- Header dumping for debugging

### Enhancements for SunlightOS
- Rust-based implementation with type safety
- Mutex-protected global state for multicore systems
- Integrated power control syscall
- Shell commands for user-friendly power management
- Comprehensive inline documentation
- Better error handling and logging

## Future Enhancements

### Phase 5.12 and Beyond
1. **User-space acpid daemon**: Move AML interpretation to Ring 3
2. **Battery monitoring**: Parse and expose battery status through VFS
3. **Power profiles**: Support different CPU power states (P-states)
4. **Thermal management**: Monitor and respond to thermal events
5. **APIC initialization**: Use MADT for multiprocessor management
6. **Wake events**: Handle power button and other wake sources

## Testing

### QEMU/KVM Verification
```bash
# Build and run
./tools/build.sh

# In QEMU console
user@sunlightos:~$ shutdown
# System cleanly powers down

user@sunlightos:~$ reboot
# System cleanly reboots
```

### Known Limitations
- Full AML interpretation not implemented (future phase)
- Battery/thermal info not exposed (future enhancement)
- Legacy ACPI 1.0 fields may not work on very old hardware
- No UEFI ACPI table extensions

## References

- **ACPI 6.4 Specification**: https://uefi.org/specs/ACPI/6.4/
- **Lux Kernel ACPI**: https://github.com/omarelghoul/lux/tree/main/kernel/src/acpi
- **Limine Bootloader**: https://limine-bootloader.org/
- **x86_64 I/O Ports**: Intel 64 and IA-32 Architectures Software Developer's Manual

## Code Statistics

- **Lines of Code**: ~600 (acpi.rs)
- **Syscall Overhead**: 1 new syscall type (PowerCtl)
- **Shell Commands**: 2 new builtins (shutdown, reboot)
- **Zero Heap Allocations**: All tables accessed through pointers to bootloader-provided memory
