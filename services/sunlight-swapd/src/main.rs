#![no_std]
#![no_main]

use sunlight_ipc::{
    debug_log, swap_active_pool_count, swap_aggregate_diagnostics, swap_configure,
    swap_online_cpu_count, sysinfo, ProcessExit,
};

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    ProcessExit::exit(1)
}

#[no_mangle]
pub extern "C" fn _start() -> ! {
    debug_log("[SWAP-1] sunlight-swapd calculating fixed boot policy");
    let info = sysinfo();
    let Some(ram_bytes) = info.total_ram_kb.checked_mul(1024) else {
        debug_log("[SWAP-1] policy failed: RAM conversion overflow");
        ProcessExit::exit(1);
    };
    let cpus = swap_online_cpu_count();
    let Ok(policy) = sunlight_ipc::swap_policy::calculate(ram_bytes, cpus) else {
        debug_log("[SWAP-1] policy failed: unsupported system information");
        ProcessExit::exit(1);
    };
    if !swap_configure(&policy) {
        debug_log("[SWAP-1] kernel rejected SwapAdmin configuration");
        ProcessExit::exit(1);
    }
    if swap_active_pool_count() != policy.pool_count {
        debug_log("[SWAP-1] health check failed: active pool count mismatch");
        ProcessExit::exit(1);
    }
    let Some(health) = swap_aggregate_diagnostics() else {
        debug_log("[SWAP-1] health snapshot unavailable");
        ProcessExit::exit(1);
    };
    if health.active_pool_count != policy.pool_count as u64 || health.service_configured == 0 {
        debug_log("[SWAP-1] health snapshot is inconsistent");
        ProcessExit::exit(1);
    }
    debug_log("[SWAP-1] policy configured; fault path is kernel-resident");
    // SWAP-1 configuration is immutable. Exiting proves service lifetime is
    // not a dependency of reclaim or swap-in.
    ProcessExit::exit(0)
}
