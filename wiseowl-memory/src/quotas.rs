//! Quota configuration and accounting for the short-term memory service.
//!
//! Defaults target low-memory hardware (see docs/MEMORY_BUDGET.md guidance).
//! All limits are hard; client-supplied sizes never drive unchecked allocations.

/// Configurable hard limits. Conservative defaults for a 512 MiB class system.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
pub struct QuotaConfig {
    /// Total RAM used by working + hot payloads and metadata buffers.
    pub total_service_ram_bytes: u64,
    /// Total cold spill budget (compressed on disk/cache).
    pub total_cold_spill_bytes: u64,
    /// Maximum single entry payload.
    pub max_entry_size: u32,
    /// Maximum sealed segment uncompressed size before forced seal.
    pub max_segment_size: u32,
    /// Maximum live entries (all classes).
    pub max_entries: u32,
    /// Maximum concurrent sessions.
    pub max_sessions: u32,
    /// Per-session RAM budget (working + hot).
    pub per_session_ram_bytes: u64,
    /// Per-session cold compressed budget.
    pub per_session_cold_bytes: u64,
    /// Maximum provenance parents (must match contract).
    pub max_provenance_parents: u32,
    /// Maximum results returned by a list query.
    pub max_list_results: u32,
    /// Maximum decompressed segment size (attacker-controlled header guard).
    pub max_decompress_bytes: u32,
}

impl Default for QuotaConfig {
    fn default() -> Self {
        Self {
            // ~2 MiB total service RAM for short-term cognitive memory.
            total_service_ram_bytes: 2 * 1024 * 1024,
            // ~4 MiB cold spill.
            total_cold_spill_bytes: 4 * 1024 * 1024,
            max_entry_size: 64 * 1024,
            max_segment_size: 256 * 1024,
            max_entries: 512,
            max_sessions: 32,
            per_session_ram_bytes: 256 * 1024,
            per_session_cold_bytes: 512 * 1024,
            max_provenance_parents: crate::provenance::MAX_PROVENANCE_PARENTS as u32,
            max_list_results: 64,
            max_decompress_bytes: 256 * 1024,
        }
    }
}

/// Live accounting snapshot (saturating counters).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
pub struct QuotaSnapshot {
    pub working_bytes: u64,
    pub hot_bytes: u64,
    pub cold_compressed_bytes: u64,
    pub cold_uncompressed_logical_bytes: u64,
    pub entry_count: u32,
    pub segment_count: u32,
    pub session_count: u32,
}

impl QuotaSnapshot {
    pub fn ram_bytes(&self) -> u64 {
        self.working_bytes.saturating_add(self.hot_bytes)
    }

    pub fn checked_add_ram(
        &self,
        add: u64,
        cfg: &QuotaConfig,
    ) -> Result<(), crate::error::MemoryError> {
        let next = self.ram_bytes().checked_add(add).ok_or(
            crate::error::MemoryError::InternalInvariantViolation("ram overflow"),
        )?;
        if next > cfg.total_service_ram_bytes {
            return Err(crate::error::MemoryError::QuotaExceeded("total service RAM"));
        }
        Ok(())
    }

    pub fn checked_add_cold(
        &self,
        add: u64,
        cfg: &QuotaConfig,
    ) -> Result<(), crate::error::MemoryError> {
        let next = self.cold_compressed_bytes.checked_add(add).ok_or(
            crate::error::MemoryError::InternalInvariantViolation("cold overflow"),
        )?;
        if next > cfg.total_cold_spill_bytes {
            return Err(crate::error::MemoryError::QuotaExceeded("total cold spill"));
        }
        Ok(())
    }

    pub fn within_limits(&self, cfg: &QuotaConfig) -> bool {
        self.ram_bytes() <= cfg.total_service_ram_bytes
            && self.cold_compressed_bytes <= cfg.total_cold_spill_bytes
            && self.entry_count <= cfg.max_entries
            && self.session_count <= cfg.max_sessions
            && self.segment_count <= cfg.max_entries // segments cannot exceed entries
    }
}

/// Per-session accounting.
#[derive(Debug, Clone, Default)]
pub struct SessionQuota {
    pub ram_bytes: u64,
    pub cold_bytes: u64,
    pub entry_count: u32,
}

impl SessionQuota {
    pub fn can_add_ram(&self, add: u64, cfg: &QuotaConfig) -> Result<(), crate::error::MemoryError> {
        let next = self
            .ram_bytes
            .checked_add(add)
            .ok_or(crate::error::MemoryError::InternalInvariantViolation(
                "session ram overflow",
            ))?;
        if next > cfg.per_session_ram_bytes {
            return Err(crate::error::MemoryError::SessionQuotaExceeded);
        }
        Ok(())
    }

    pub fn can_add_cold(
        &self,
        add: u64,
        cfg: &QuotaConfig,
    ) -> Result<(), crate::error::MemoryError> {
        let next = self
            .cold_bytes
            .checked_add(add)
            .ok_or(crate::error::MemoryError::InternalInvariantViolation(
                "session cold overflow",
            ))?;
        if next > cfg.per_session_cold_bytes {
            return Err(crate::error::MemoryError::SessionQuotaExceeded);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_bounded() {
        let c = QuotaConfig::default();
        assert!(c.total_service_ram_bytes > 0);
        assert!(c.max_entry_size <= c.max_segment_size);
        assert!(c.max_list_results > 0);
        assert!(c.max_decompress_bytes >= c.max_segment_size);
    }

    #[test]
    fn ram_quota_rejects() {
        let cfg = QuotaConfig {
            total_service_ram_bytes: 100,
            ..QuotaConfig::default()
        };
        let snap = QuotaSnapshot {
            working_bytes: 90,
            ..Default::default()
        };
        assert!(snap.checked_add_ram(20, &cfg).is_err());
        assert!(snap.checked_add_ram(10, &cfg).is_ok());
    }
}
