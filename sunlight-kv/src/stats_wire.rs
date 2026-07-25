//! Bounded, versioned diagnostic snapshot wire format for sunlight-kv.
//!
//! Layout is a fixed little-endian sequence of `u64` fields so the service can
//! write it with no formatting or heap allocation, and `sunlight-kvctl stats`
//! can decode and print it on the client side.
//!
//! This module is pure `core` so it builds for both host and SunlightOS.

/// Wire magic: ASCII "KVST" in the low 32 bits.
pub const STATS_MAGIC: u64 = 0x5453_564B; // 'K''V''S''T' LE
/// Schema version for this fixed field layout.
pub const STATS_VERSION: u64 = 1;
/// Number of fixed `u64` fields before the client attribution table.
pub const STATS_FIXED_U64S: usize = 48;
/// Fixed number of client attribution slots (pid + request count each).
pub const STATS_CLIENT_SLOTS: usize = 16;
/// Total `u64` words in a v1 snapshot (fixed section + client table).
pub const STATS_TOTAL_U64S: usize = STATS_FIXED_U64S + STATS_CLIENT_SLOTS * 2;
/// Byte length of a complete v1 snapshot.
pub const STATS_BYTES: usize = STATS_TOTAL_U64S * 8;

// Fixed-field indices (must stay in sync with encode/decode and CLI labels).
pub const F_MAGIC: usize = 0;
pub const F_VERSION: usize = 1;
pub const F_STARTED_MS: usize = 2;
pub const F_NOW_MS: usize = 3;
pub const F_LAST_ACTIVITY_MS: usize = 4;
pub const F_LAST_ERROR_MS: usize = 5;
pub const F_LAST_ERROR_CODE: usize = 6;
pub const F_VOLATILE_ONLY: usize = 7;

pub const F_REQUESTS_TOTAL: usize = 8;
pub const F_REQUESTS_OK: usize = 9;
pub const F_REQUESTS_ERR: usize = 10;
pub const F_DECODE_ERRORS: usize = 11;
pub const F_REPLY_ERROR_LABELS: usize = 12;
pub const F_UNKNOWN_OPCODES: usize = 13;

pub const F_OP_PUT: usize = 14;
pub const F_OP_GET: usize = 15;
pub const F_OP_DELETE: usize = 16;
pub const F_OP_SCAN: usize = 17;
pub const F_OP_PUT_SHM: usize = 18;
pub const F_OP_GET_SHM: usize = 19;
pub const F_OP_PUT_SHM2: usize = 20;
pub const F_OP_GET_SHM2: usize = 21;
pub const F_OP_DELETE_SHM2: usize = 22;
pub const F_OP_STATS: usize = 23;

pub const F_LOOP_ITERATIONS: usize = 24;
pub const F_RECV_BLOCKING: usize = 25;
pub const F_TRY_RECV_HIT: usize = 26;
pub const F_TRY_RECV_MISS: usize = 27;

pub const F_KEY_COUNT: usize = 28;
pub const F_PAYLOAD_BYTES: usize = 29;
pub const F_MUTATIONS: usize = 30;
pub const F_PERSIST_QUEUE_DEPTH: usize = 31;
pub const F_PERSIST_QUEUE_HWM: usize = 32;
pub const F_PERSIST_FLUSH_OK: usize = 33;
pub const F_PERSIST_FLUSH_FAIL: usize = 34;
pub const F_PERSIST_RECORD_BYTES: usize = 35;
pub const F_PERSIST_SKIPPED_VOLATILE: usize = 36;

pub const F_CLIENT_SLOTS_USED: usize = 37;
pub const F_OTHER_CLIENT_REQUESTS: usize = 38;
pub const F_CLIENT_PIDS_TRACKED: usize = 39;

// Reserved fixed slots 40..47 for future additive fields within v1 size.
#[allow(dead_code)]
pub const F_RESERVED_0: usize = 40;

/// First index of the client table: pairs of (pid, requests).
pub const F_CLIENT_TABLE: usize = STATS_FIXED_U64S;

/// Read-only view of a decoded v1 snapshot.
#[derive(Clone, Copy, Debug)]
pub struct StatsSnapshotV1 {
    pub words: [u64; STATS_TOTAL_U64S],
}

impl StatsSnapshotV1 {
    pub const fn zeroed() -> Self {
        Self {
            words: [0; STATS_TOTAL_U64S],
        }
    }

    #[inline]
    pub fn get(&self, idx: usize) -> u64 {
        self.words.get(idx).copied().unwrap_or(0)
    }

    #[inline]
    pub fn set(&mut self, idx: usize, val: u64) {
        if let Some(slot) = self.words.get_mut(idx) {
            *slot = val;
        }
    }

    /// Encode as little-endian bytes into `out`. Returns bytes written, or
    /// `None` if `out` is too small.
    pub fn encode(&self, out: &mut [u8]) -> Option<usize> {
        if out.len() < STATS_BYTES {
            return None;
        }
        for (i, word) in self.words.iter().enumerate() {
            let off = i * 8;
            out[off..off + 8].copy_from_slice(&word.to_le_bytes());
        }
        Some(STATS_BYTES)
    }

    /// Decode from little-endian bytes. Returns `None` on short buffer,
    /// bad magic, or unsupported version.
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < STATS_BYTES {
            return None;
        }
        let mut snap = Self::zeroed();
        for i in 0..STATS_TOTAL_U64S {
            let off = i * 8;
            let mut le = [0u8; 8];
            le.copy_from_slice(&bytes[off..off + 8]);
            snap.words[i] = u64::from_le_bytes(le);
        }
        if snap.get(F_MAGIC) != STATS_MAGIC {
            return None;
        }
        if snap.get(F_VERSION) != STATS_VERSION {
            return None;
        }
        Some(snap)
    }

    pub fn client_pid(&self, slot: usize) -> u64 {
        if slot >= STATS_CLIENT_SLOTS {
            return 0;
        }
        self.get(F_CLIENT_TABLE + slot * 2)
    }

    pub fn client_requests(&self, slot: usize) -> u64 {
        if slot >= STATS_CLIENT_SLOTS {
            return 0;
        }
        self.get(F_CLIENT_TABLE + slot * 2 + 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_preserves_fields() {
        let mut s = StatsSnapshotV1::zeroed();
        s.set(F_MAGIC, STATS_MAGIC);
        s.set(F_VERSION, STATS_VERSION);
        s.set(F_REQUESTS_TOTAL, 42);
        s.set(F_OP_PUT, 7);
        s.set(F_CLIENT_TABLE, 1001);
        s.set(F_CLIENT_TABLE + 1, 9);
        let mut buf = [0u8; STATS_BYTES];
        assert_eq!(s.encode(&mut buf), Some(STATS_BYTES));
        let d = StatsSnapshotV1::decode(&buf).expect("decode");
        assert_eq!(d.get(F_REQUESTS_TOTAL), 42);
        assert_eq!(d.get(F_OP_PUT), 7);
        assert_eq!(d.client_pid(0), 1001);
        assert_eq!(d.client_requests(0), 9);
    }

    #[test]
    fn reject_bad_magic_or_short() {
        let mut buf = [0u8; STATS_BYTES];
        assert!(StatsSnapshotV1::decode(&buf).is_none());
        let mut s = StatsSnapshotV1::zeroed();
        s.set(F_MAGIC, STATS_MAGIC);
        s.set(F_VERSION, 99);
        s.encode(&mut buf);
        assert!(StatsSnapshotV1::decode(&buf).is_none());
        assert!(StatsSnapshotV1::decode(&buf[..8]).is_none());
    }

    #[test]
    fn size_fits_one_shm_page() {
        assert!(STATS_BYTES <= 4096);
        assert_eq!(STATS_BYTES, (48 + 16 * 2) * 8);
    }
}
