// Stub for sunlight-compat-linux (Helios Subsystem)
#![no_std]

pub fn translate_syscall(_nr: u64, _args: [u64; 6]) -> i64 {
    // TODO: translate Linux syscall to IPC message
    -38 // ENOSYS
}
