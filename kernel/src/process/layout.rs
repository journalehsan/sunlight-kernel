pub const USER_STACK_TOP: u64 = 0x0000_7FFF_FFFF_F000;
pub const USER_STACK_SIZE: u64 = 512 * 1024;
pub const USER_HEAP_START: u64 = 0x0000_0001_0000_0000;
pub const USER_CODE_START: u64 = 0x0000_0000_0040_0000;
pub const KERNEL_START: u64 = 0xFFFF_FFFF_8000_0000;

/// Check if a virtual address is in user space.
pub fn is_user_address(addr: u64) -> bool {
    addr < KERNEL_START
}
