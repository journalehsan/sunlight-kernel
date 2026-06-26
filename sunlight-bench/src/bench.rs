/// Core benchmark trait. `run` returns elapsed TSC cycles for the workload.
pub trait Benchmark {
    fn name(&self) -> &'static str;
    fn run(&self) -> u64;
}

/// Serializing RDTSC: LFENCE drains the out-of-order pipeline first.
#[inline(always)]
pub fn rdtsc() -> u64 {
    let lo: u32;
    let hi: u32;
    unsafe {
        core::arch::asm!(
            "lfence",
            "rdtsc",
            out("eax") lo,
            out("edx") hi,
            options(nomem, nostack, preserves_flags),
        );
    }
    ((hi as u64) << 32) | lo as u64
}
