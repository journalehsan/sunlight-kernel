pub const USER_STACK_TOP: u64 = 0x0000_7FFF_FFFF_F000;
// 2 MiB: sunlightd's `_start` frame alone is ~516 KiB (it holds ServiceTable,
// DepGraph and BootStartup by value), and its call into parse_service_unit
// then needs another ~8 KiB. The old 512 KiB stack was crossed by 784 bytes,
// faulting sunlightd (pid 7) at boot and breaking every supervised service.
pub const USER_STACK_SIZE: u64 = 2 * 1024 * 1024;
pub const USER_HEAP_START: u64 = 0x0000_0001_0000_0000;
pub const USER_CODE_START: u64 = 0x0000_0000_0040_0000;
/// Fixed user VA for `SYS_MAP_TELEMETRY` (read-only kernel TelemetryPage).
///
/// Must not sit inside typical ELF PT_LOAD ranges. Code starts at
/// `USER_CODE_START` (4 MiB); large shells such as sunlight-vortex-shell embed
/// multi-MiB `.rodata` that already extend past the historical fixed slot at
/// 8 MiB (`0x0080_0000`), which made `Telemetry::init()` fail with a mapping
/// collision and left the Sidebar System Monitor on "Telemetry unavailable".
/// 2 GiB is well above asset-heavy binaries, below the user stack window, and
/// outside the anonymous mmap base (`0x10_0000_0000`).
pub const USER_TELEMETRY_BASE: u64 = 0x0000_0000_7E00_0000;
pub use crate::memory::user::USER_END_EXCLUSIVE as KERNEL_START;

/// Check if a virtual address is in user space.
pub fn is_user_address(addr: u64) -> bool {
    crate::memory::user::UserAddress::new(addr).is_ok()
}
