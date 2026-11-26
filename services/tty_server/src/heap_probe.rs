use core::fmt::{self, Write};

#[derive(Default)]
struct Line {
    bytes: [u8; 32],
    len: usize,
}

impl Write for Line {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        let end = self.len + value.len();
        self.bytes
            .get_mut(self.len..end)
            .ok_or(fmt::Error)?
            .copy_from_slice(value.as_bytes());
        self.len = end;
        Ok(())
    }
}

pub(super) fn sample(stage: &str, tabs: usize) {
    let stats = sunlight_libc::alloc::heap_stats();
    let mut bytes = [0u8; 256];
    let mut len = 0;
    for value in [
        sunlight_ipc::monotonic_millis() as usize,
        tabs,
        stats.heap_capacity,
        stats.requested_user_bytes,
        stats.allocated_backing_bytes,
        stats.high_water_allocated_bytes,
        stats.failed_allocation_count as usize,
    ] {
        let mut field = Line::default();
        if write!(field, " {value}").is_err() {
            return;
        }
        bytes[len..len + field.len].copy_from_slice(&field.bytes[..field.len]);
        len += field.len;
    }
    sunlight_ipc::debug_log("[TTY-HEAP] ");
    sunlight_ipc::debug_log(stage);
    sunlight_ipc::debug_log(core::str::from_utf8(&bytes[..len]).unwrap_or(""));
    sunlight_ipc::debug_log("\n");
}

pub(super) fn check_growth() {
    use alloc::vec::Vec;
    use sunlight_libc::alloc::heap_stats;

    let before = heap_stats();
    let mut buffer = Vec::<u8>::new();
    if buffer.try_reserve_exact(1024 * 1024).is_err() {
        sunlight_ipc::debug_log("[TTY-HEAP] growth probe FAIL: reserve");
        return;
    }
    buffer.resize(1024 * 1024, 0xa5);
    let grown = heap_stats();
    let rejected = buffer.try_reserve_exact(16 * 1024 * 1024).is_err();
    let preserved = buffer.len() == 1024 * 1024 && buffer.iter().all(|&byte| byte == 0xa5);
    drop(buffer);
    let after = heap_stats();
    sample("growth-recovered", 0);
    if grown.heap_capacity > before.heap_capacity
        && rejected
        && preserved
        && after.heap_capacity == before.heap_capacity
        && after.requested_user_bytes == before.requested_user_bytes
        && after.allocated_backing_bytes == before.allocated_backing_bytes
        && after.failed_allocation_count == before.failed_allocation_count + 1
    {
        sunlight_ipc::debug_log("[TTY-HEAP] growth probe PASS");
    } else {
        sunlight_ipc::debug_log("[TTY-HEAP] growth probe FAIL: accounting or rejection");
    }
}
