use core::sync::atomic::{AtomicU64, AtomicU32, Ordering};

#[derive(Debug)]
pub struct BrainDiagnostics {
    pub requests_total: AtomicU64,
    pub requests_greeting: AtomicU64,
    pub requests_rejected: AtomicU64,
    pub requests_failed: AtomicU64,
    pub requests_timed_out: AtomicU64,
    pub context_build_failures: AtomicU64,
    pub provider_local_used: AtomicU64,
    pub provider_fallback_used: AtomicU64,
    pub provider_future_used: AtomicU64,
    pub responses_successful: AtomicU64,
    pub response_alignment_failures: AtomicU64,
    pub welcome_client_requests: AtomicU64,
    pub welcome_client_fallbacks: AtomicU64,
    pub context_kv_success: AtomicU64,
    pub context_kv_degraded: AtomicU64,
    pub context_memorydb_success: AtomicU64,
    pub context_memorydb_degraded: AtomicU64,
    pub context_index_success: AtomicU64,
    pub context_index_degraded: AtomicU64,
    pub kv_reads: AtomicU64,
    pub kv_read_failures: AtomicU64,
    pub kv_writes: AtomicU64,
    pub kv_write_failures: AtomicU64,
    pub responses_first_visit: AtomicU64,
    pub responses_returning_visit: AtomicU64,
    pub responses_after_upgrade: AtomicU64,
    pub responses_with_machine_summary: AtomicU64,
    pub responses_with_index_status: AtomicU64,
    pub unauthorized_requests: AtomicU64,
    pub foundation_load_success: AtomicU64,
    pub foundation_load_failures: AtomicU64,
    pub foundation_record_count: AtomicU64,
    pub foundation_token_count: AtomicU64,
    pub last_error_code: AtomicU32,
}

impl BrainDiagnostics {
    pub const fn new() -> Self {
        Self {
            requests_total: AtomicU64::new(0),
            requests_greeting: AtomicU64::new(0),
            requests_rejected: AtomicU64::new(0),
            requests_failed: AtomicU64::new(0),
            requests_timed_out: AtomicU64::new(0),
            context_build_failures: AtomicU64::new(0),
            provider_local_used: AtomicU64::new(0),
            provider_fallback_used: AtomicU64::new(0),
            provider_future_used: AtomicU64::new(0),
            responses_successful: AtomicU64::new(0),
            response_alignment_failures: AtomicU64::new(0),
            welcome_client_requests: AtomicU64::new(0),
            welcome_client_fallbacks: AtomicU64::new(0),
            context_kv_success: AtomicU64::new(0),
            context_kv_degraded: AtomicU64::new(0),
            context_memorydb_success: AtomicU64::new(0),
            context_memorydb_degraded: AtomicU64::new(0),
            context_index_success: AtomicU64::new(0),
            context_index_degraded: AtomicU64::new(0),
            kv_reads: AtomicU64::new(0),
            kv_read_failures: AtomicU64::new(0),
            kv_writes: AtomicU64::new(0),
            kv_write_failures: AtomicU64::new(0),
            responses_first_visit: AtomicU64::new(0),
            responses_returning_visit: AtomicU64::new(0),
            responses_after_upgrade: AtomicU64::new(0),
            responses_with_machine_summary: AtomicU64::new(0),
            responses_with_index_status: AtomicU64::new(0),
            unauthorized_requests: AtomicU64::new(0),
            foundation_load_success: AtomicU64::new(0),
            foundation_load_failures: AtomicU64::new(0),
            foundation_record_count: AtomicU64::new(0),
            foundation_token_count: AtomicU64::new(0),
            last_error_code: AtomicU32::new(0),
        }
    }

    pub fn inc_requests(&self) { self.requests_total.fetch_add(1, Ordering::Relaxed); }
    pub fn inc_greeting(&self) { self.requests_greeting.fetch_add(1, Ordering::Relaxed); }
    pub fn inc_rejected(&self) { self.requests_rejected.fetch_add(1, Ordering::Relaxed); }
    pub fn inc_failed(&self) { self.requests_failed.fetch_add(1, Ordering::Relaxed); }
    pub fn inc_timed_out(&self) { self.requests_timed_out.fetch_add(1, Ordering::Relaxed); }
    pub fn inc_context_fail(&self) { self.context_build_failures.fetch_add(1, Ordering::Relaxed); }
    pub fn inc_local(&self) { self.provider_local_used.fetch_add(1, Ordering::Relaxed); }
    pub fn inc_fallback(&self) { self.provider_fallback_used.fetch_add(1, Ordering::Relaxed); }
    pub fn inc_future(&self) { self.provider_future_used.fetch_add(1, Ordering::Relaxed); }
    pub fn inc_success(&self) { self.responses_successful.fetch_add(1, Ordering::Relaxed); }
    pub fn inc_alignment_fail(&self) { self.response_alignment_failures.fetch_add(1, Ordering::Relaxed); }
    pub fn inc_welcome(&self) { self.welcome_client_requests.fetch_add(1, Ordering::Relaxed); }
    pub fn inc_welcome_fallback(&self) { self.welcome_client_fallbacks.fetch_add(1, Ordering::Relaxed); }
    pub fn inc_first_visit(&self) { self.responses_first_visit.fetch_add(1, Ordering::Relaxed); }
    pub fn inc_returning_visit(&self) { self.responses_returning_visit.fetch_add(1, Ordering::Relaxed); }
    pub fn inc_after_upgrade(&self) { self.responses_after_upgrade.fetch_add(1, Ordering::Relaxed); }
    pub fn inc_machine_summary(&self) { self.responses_with_machine_summary.fetch_add(1, Ordering::Relaxed); }
    pub fn inc_index_status(&self) { self.responses_with_index_status.fetch_add(1, Ordering::Relaxed); }
    pub fn inc_kv_success(&self) { self.context_kv_success.fetch_add(1, Ordering::Relaxed); }
    pub fn inc_kv_degraded(&self) { self.context_kv_degraded.fetch_add(1, Ordering::Relaxed); }
    pub fn inc_memorydb_success(&self) { self.context_memorydb_success.fetch_add(1, Ordering::Relaxed); }
    pub fn inc_memorydb_degraded(&self) { self.context_memorydb_degraded.fetch_add(1, Ordering::Relaxed); }
    pub fn inc_index_success(&self) { self.context_index_success.fetch_add(1, Ordering::Relaxed); }
    pub fn inc_index_degraded(&self) { self.context_index_degraded.fetch_add(1, Ordering::Relaxed); }
    pub fn inc_kv_read(&self) { self.kv_reads.fetch_add(1, Ordering::Relaxed); }
    pub fn inc_kv_read_fail(&self) { self.kv_read_failures.fetch_add(1, Ordering::Relaxed); }
    pub fn inc_kv_write(&self) { self.kv_writes.fetch_add(1, Ordering::Relaxed); }
    pub fn inc_kv_write_fail(&self) { self.kv_write_failures.fetch_add(1, Ordering::Relaxed); }
    pub fn inc_unauthorized(&self) { self.unauthorized_requests.fetch_add(1, Ordering::Relaxed); }
    pub fn note_foundation_loaded(&self, records: u64, tokens: u64) {
        self.foundation_load_success.fetch_add(1, Ordering::Relaxed);
        self.foundation_record_count.store(records, Ordering::Relaxed);
        self.foundation_token_count.store(tokens, Ordering::Relaxed);
    }
    pub fn note_foundation_failed(&self) { self.foundation_load_failures.fetch_add(1, Ordering::Relaxed); }
    pub fn set_error(&self, code: u16) { self.last_error_code.store(code as u32, Ordering::Relaxed); }

    pub fn provider_local_available(&self) -> bool {
        true
    }

    pub fn provider_future_available(&self) -> bool {
        false
    }

    pub fn requests_active(&self) -> u32 {
        0
    }
}

#[derive(Debug, Clone)]
pub struct BrainHealthSnapshot {
    pub requests_total: u64,
    pub requests_active: u32,
    pub requests_failed: u64,
    pub last_error_code: Option<u16>,
    pub provider_local_available: bool,
    pub provider_future_available: bool,
}

impl BrainDiagnostics {
    pub fn snapshot(&self) -> BrainHealthSnapshot {
        let code = self.last_error_code.load(Ordering::Relaxed);
        BrainHealthSnapshot {
            requests_total: self.requests_total.load(Ordering::Relaxed),
            requests_active: self.requests_active(),
            requests_failed: self.requests_failed.load(Ordering::Relaxed),
            last_error_code: if code == 0 { None } else { Some(code as u16) },
            provider_local_available: self.provider_local_available(),
            provider_future_available: self.provider_future_available(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counters_increment() {
        let d = BrainDiagnostics::new();
        d.inc_requests();
        d.inc_greeting();
        d.inc_local();
        d.inc_success();
        assert_eq!(d.requests_total.load(Ordering::Relaxed), 1);
        assert_eq!(d.requests_greeting.load(Ordering::Relaxed), 1);
        assert_eq!(d.provider_local_used.load(Ordering::Relaxed), 1);
        assert_eq!(d.responses_successful.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn snapshot_reflects_state() {
        let d = BrainDiagnostics::new();
        d.inc_requests();
        d.inc_greeting();
        d.set_error(42);
        let snap = d.snapshot();
        assert_eq!(snap.requests_total, 1);
        assert_eq!(snap.last_error_code, Some(42));
        assert!(snap.provider_local_available);
        assert!(!snap.provider_future_available);
    }

    #[test]
    fn provider_local_always_available() {
        let d = BrainDiagnostics::new();
        assert!(d.provider_local_available());
    }

    #[test]
    fn provider_future_not_available() {
        let d = BrainDiagnostics::new();
        assert!(!d.provider_future_available());
    }

    #[test]
    fn counters_bounded() {
        let d = BrainDiagnostics::new();
        for _ in 0..100 {
            d.inc_requests();
            d.inc_greeting();
        }
        assert_eq!(d.requests_total.load(Ordering::Relaxed), 100);
        assert_eq!(d.requests_greeting.load(Ordering::Relaxed), 100);
    }
}
