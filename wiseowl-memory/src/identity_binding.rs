//! Runtime-only authorization gate for durable memory promotion.

pub const NATIVE_STATUS_REPLY: u64 = 0x4D80;
const IDENTITY_STATUS_VERSION: u64 = 2;
const STATUS_READY: u64 = 1;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MemoryIdentityBinding {
    fingerprint: Option<u64>,
    ready: bool,
    mismatch: bool,
    activation_fingerprint: Option<u64>,
    connected: bool,
    endpoint_generation: Option<u64>,
    activation_suspended: bool,
}

impl MemoryIdentityBinding {
    /// Mark the current endpoint unavailable. Keep the pin only for mismatch detection.
    pub fn disconnected(&mut self) {
        self.ready = false;
        // A missing reply does not prove a new endpoint incarnation.
    }

    pub fn observe_endpoint_generation(&mut self, generation: u64) {
        if self.endpoint_generation.is_some_and(|old| old != generation) {
            self.connected = false;
            self.ready = false;
            self.activation_suspended = false;
        }
        self.endpoint_generation = Some(generation);
    }

    pub fn observe(&mut self, valid_ready_status: bool, fingerprint: u64) -> bool {
        self.ready = false;
        if self.mismatch {
            return false;
        }
        if !valid_ready_status || fingerprint == 0 {
            return false;
        }
        if self.fingerprint.is_some_and(|old| old != fingerprint) {
            self.mismatch = true;
            return false;
        }
        self.fingerprint = Some(fingerprint);
        self.ready = true;
        true
    }

    /// Decode the native fixed-word representation. Invalid replies revoke readiness.
    pub fn observe_native_reply(&mut self, label: u64, word_count: u32, words: [u64; 8]) -> bool {
        self.ready = false;
        let header = words[0];
        if self.mismatch
            || self.activation_suspended
            || label != NATIVE_STATUS_REPLY
            || word_count != 4
            || (header & 0xff) != STATUS_READY
            || ((header >> 8) & 0xffff) != IDENTITY_STATUS_VERSION
            || ((header >> 24) & 1) != 1
            || !matches!((header >> 25) & 0xff, 1 | 2)
            || ((header >> 33) & 0xffff) != 1
            || ((header >> 49) & 0xff) != 1
            || header >> 60 != 0
            || ((header >> 57) & 0x7) != 3
            || words[2] == 0
            || (words[3] & 0xffff_ffff) == 0
            || (words[3] >> 32) == 0
        {
            return false;
        }
        let fingerprint = words[1];
        if fingerprint == 0 {
            return false;
        }
        if self.fingerprint.is_some_and(|old| old != fingerprint) {
            self.mismatch = true;
            return false;
        }
        let activation = words[2];
        if self.connected && self.activation_fingerprint.is_some_and(|old| old != activation) {
            self.activation_suspended = true;
            return false;
        }
        self.fingerprint = Some(fingerprint);
        self.activation_fingerprint = Some(activation);
        self.connected = true;
        self.ready = true;
        true
    }

    pub const fn durable_promotion_allowed(&self) -> bool {
        self.ready && !self.mismatch
    }

    pub const fn mismatch_detected(&self) -> bool {
        self.mismatch
    }

    pub const fn fingerprint(&self) -> Option<u64> {
        self.fingerprint
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn native(fp: u64) -> [u64; 8] {
        [
            STATUS_READY
                | (IDENTITY_STATUS_VERSION << 8)
                | (1 << 24)
                | (1 << 25)
                | (1 << 33)
                | (1 << 49)
                | (3 << 57),
            fp,
            u64::from_le_bytes(*b"ACTIV001"),
            (1 << 32) | 1,
            0,
            0,
            0,
            0,
        ]
    }

    #[test]
    fn promotion_gate_handles_valid_disconnect_reconnect_and_mismatch() {
        let mut gate = MemoryIdentityBinding::default();
        assert!(gate.observe(true, u64::from_le_bytes(*b"IDENT-A1")));
        assert!(gate.durable_promotion_allowed());
        gate.disconnected();
        assert!(!gate.durable_promotion_allowed());
        assert!(gate.observe(true, u64::from_le_bytes(*b"IDENT-A1")));
        assert!(gate.durable_promotion_allowed());
        gate.disconnected();
        assert!(!gate.observe(true, u64::from_le_bytes(*b"IDENT-B1")));
        assert!(gate.mismatch_detected());
        assert!(!gate.durable_promotion_allowed());
    }

    #[test]
    fn invalid_unready_and_malformed_status_fail_closed() {
        let mut gate = MemoryIdentityBinding::default();
        assert!(!gate.observe(false, 0));
        assert!(!gate.observe(false, u64::from_le_bytes(*b"IDENT-A1")));
        assert!(!gate.durable_promotion_allowed());
        assert!(!gate.observe_native_reply(NATIVE_STATUS_REPLY, 4, native(0)));
        assert!(!gate.observe_native_reply(NATIVE_STATUS_REPLY + 1, 4, native(1)));
        assert!(!gate.observe_native_reply(NATIVE_STATUS_REPLY, 5, native(1)));
        let mut malformed = native(1);
        malformed[0] &= !(0x7 << 57);
        malformed[0] |= 2 << 57;
        assert!(!gate.observe_native_reply(NATIVE_STATUS_REPLY, 4, malformed));
        let mut wrong_version = native(1);
        wrong_version[0] ^= 1 << 8;
        assert!(!gate.observe_native_reply(NATIVE_STATUS_REPLY, 4, wrong_version));
    }

    #[cfg(feature = "host")]
    #[test]
    fn unavailable_identity_denies_durable_promotion_but_keeps_ram_create_available() {
        use crate::caller::CallerIdentity;
        use crate::kinds::{MemoryClass, MemoryKind, SourceKind, TrustLevel};
        use crate::protocol::{ProtocolRequest, ProtocolResponse, PROTOCOL_VERSION};
        use crate::provenance::Provenance;
        use crate::service::{MemoryService, ServiceConfig};

        let gate = MemoryIdentityBinding::default();
        assert!(!gate.durable_promotion_allowed());

        let mut service = MemoryService::new(ServiceConfig::default()).unwrap();
        let caller = CallerIdentity::admin();
        let session_id = match service.handle(
            &caller,
            ProtocolRequest::CreateSession {
                protocol_version: PROTOCOL_VERSION,
            },
        ) {
            ProtocolResponse::SessionCreated { session_id } => session_id,
            other => panic!("session creation failed: {other:?}"),
        };
        let created = service.handle(
            &caller,
            ProtocolRequest::CreateEntry {
                protocol_version: PROTOCOL_VERSION,
                session_id,
                class: MemoryClass::Working,
                kind: MemoryKind::Input,
                importance: 100,
                confidence: 100,
                ttl_ns: None,
                payload: b"RAM remains available".to_vec(),
                token_stream: None,
                provenance: Provenance::new(
                    SourceKind::UserInput,
                    None,
                    1,
                    "phase-b-test",
                    TrustLevel::Untrusted,
                ),
            },
        );
        assert!(matches!(created, ProtocolResponse::Created { .. }));
    }

    #[test]
    fn native_status_reconnect_requires_same_fingerprint() {
        let mut gate = MemoryIdentityBinding::default();
        assert!(gate.observe_native_reply(NATIVE_STATUS_REPLY, 4, native(0x3141_5926_5358_9793)));
        gate.disconnected();
        assert!(!gate.durable_promotion_allowed());
        assert!(gate.observe_native_reply(NATIVE_STATUS_REPLY, 4, native(0x3141_5926_5358_9793)));
        gate.disconnected();
        assert!(!gate.observe_native_reply(NATIVE_STATUS_REPLY, 4, native(0x2718_2818_2845_9045)));
        assert!(gate.mismatch_detected());
    }

    #[test]
    fn memoryd_restart_requeries_instead_of_restoring_local_identity_cache() {
        let status_a = native(0x3141_5926_5358_9793);
        let mut running = MemoryIdentityBinding::default();
        assert!(running.observe_native_reply(NATIVE_STATUS_REPLY, 4, status_a));
        assert!(running.durable_promotion_allowed());

        let mut restarted = MemoryIdentityBinding::default();
        assert_eq!(restarted.fingerprint(), None);
        assert!(!restarted.durable_promotion_allowed());
        assert!(restarted.observe_native_reply(NATIVE_STATUS_REPLY, 4, status_a));
        assert!(restarted.durable_promotion_allowed());
    }

    #[test]
    fn guest_native_status_sample_is_accepted() {
        let words = native(4_919_681_448_421_046_081);
        let mut gate = MemoryIdentityBinding::default();
        assert!(gate.observe_native_reply(NATIVE_STATUS_REPLY, 4, words));
    }

    #[test]
    fn activation_change_inside_connection_revokes_promotion() {
        let mut gate = MemoryIdentityBinding::default();
        gate.observe_endpoint_generation(1);
        let mut reply = native(u64::from_le_bytes(*b"IDENT-A1"));
        assert!(gate.observe_native_reply(NATIVE_STATUS_REPLY, 4, reply));
        reply[2] ^= 1;
        assert!(!gate.observe_native_reply(NATIVE_STATUS_REPLY, 4, reply));
        assert!(!gate.mismatch_detected());
        reply[2] ^= 1;
        assert!(!gate.observe_native_reply(NATIVE_STATUS_REPLY, 4, reply));
        gate.disconnected();
        reply[2] = u64::from_le_bytes(*b"ACTIV002");
        gate.observe_endpoint_generation(2);
        assert!(gate.observe_native_reply(NATIVE_STATUS_REPLY, 4, reply));
    }
}
