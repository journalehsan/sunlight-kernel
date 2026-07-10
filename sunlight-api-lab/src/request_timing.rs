pub struct RequestTimer {
    started_at_ms: u64,
}

impl RequestTimer {
    pub fn start_now() -> Self {
        Self {
            started_at_ms: sunlight_ipc::monotonic_millis(),
        }
    }

    pub fn elapsed_ms(&self) -> u64 {
        sunlight_ipc::monotonic_millis().saturating_sub(self.started_at_ms)
    }
}
