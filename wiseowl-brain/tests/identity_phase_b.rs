use wiseowl_brain::identity_binding::{BrainIdentityBinding, BrainIdentityMode};
use wiseowl_index::config::IndexerConfig;
use wiseowl_index::{IndexCaller, IndexerService};
use wiseowl_memory::identity_binding::MemoryIdentityBinding;
use wiseowl_memorydb::database::{Database, MemoryStore};
use wiseowl_memorydb::identity::{ensure_identity, AdoptionPolicy, StoreDisposition};
use wiseowl_memorydb::DbQuotaConfig;

fn new_identity(seed: u8) -> wiseowl_memorydb::identity::LoadedIdentity {
    let dir = tempfile::tempdir().unwrap();
    let mut storage =
        wiseowl_memorydb::identity::host::HostIdentityStorage::open(dir.path()).unwrap();
    ensure_identity(
        &mut storage,
        StoreDisposition::Fresh,
        AdoptionPolicy::Disabled,
        |bytes| {
            bytes.fill(seed);
            Ok(())
        },
        |_| Ok(()),
    )
    .unwrap()
}

#[test]
fn four_services_bind_reconnect_and_reject_wrong_identity() {
    let identity_a = new_identity(7);
    let mut db = Database::<MemoryStore>::open_memory(DbQuotaConfig::default()).unwrap();
    db.bind_identity_context(identity_a);
    let status_a = db.identity_status().unwrap();
    let fingerprint_a = u64::from_le_bytes(status_a.fingerprint);
    assert_eq!(status_a.continuity_generation, 1);

    let mut brain = BrainIdentityBinding::default();
    brain.observe(Some(status_a));
    assert_eq!(brain.mode, BrainIdentityMode::Personalized);

    let mut index = IndexerService::new(db, IndexerConfig::default());
    index.refresh_memorydb_health();
    let caller = IndexCaller::admin();
    assert!(index.start_scan(&caller, None).is_ok());

    let mut memory = MemoryIdentityBinding::default();
    assert!(memory.observe(status_a.validate(), fingerprint_a));
    assert!(memory.durable_promotion_allowed());
    let native_words = status_a.encode_native_words().unwrap();
    let mut native_reply = [0; 8];
    native_reply[..4].copy_from_slice(&native_words);
    let mut native_memory = MemoryIdentityBinding::default();
    assert!(native_memory.observe_native_reply(0x4D80, 4, native_reply));
    assert!(native_memory.durable_promotion_allowed());

    let records_before_disconnect = index.backend.inner().stats().record_count_active;
    index.backend.set_endpoint_available(false);
    index.refresh_memorydb_health();
    brain.disconnected();
    brain.observe(None);
    memory.disconnected();
    assert_eq!(brain.mode, BrainIdentityMode::GenericDegraded);
    assert_eq!(brain.continuity_generation, None);
    assert!(index.start_scan(&caller, None).is_err());
    assert!(!memory.durable_promotion_allowed());
    assert_eq!(
        index.backend.inner().stats().record_count_active,
        records_before_disconnect
    );

    index.backend.set_endpoint_available(true);
    index.backend.set_endpoint_generation(2);
    index.refresh_memorydb_health();
    brain.observe(index.backend.inner().identity_status());
    let status_again = index.backend.inner().identity_status().unwrap();
    assert!(memory.observe(
        status_again.validate(),
        u64::from_le_bytes(status_again.fingerprint)
    ));
    assert_eq!(brain.mode, BrainIdentityMode::Personalized);
    assert_eq!(brain.fingerprint(), Some(status_a.fingerprint));
    assert_eq!(index.health.memorydb.endpoint_generation(), 2);
    assert!(index.start_scan(&caller, None).is_ok());
    assert!(memory.durable_promotion_allowed());

    let identity_b = new_identity(19);
    index.backend.inner_mut().bind_identity_context(identity_b);
    index.refresh_memorydb_health();
    let status_b = index.backend.inner().identity_status().unwrap();
    brain.disconnected();
    brain.observe(Some(status_b));
    memory.disconnected();
    assert!(!memory.observe(
        status_b.validate(),
        u64::from_le_bytes(status_b.fingerprint)
    ));
    assert_eq!(brain.mode, BrainIdentityMode::SuspendedMismatch);
    assert!(index.start_scan(&caller, None).is_err());
    assert!(!memory.durable_promotion_allowed());
    assert!(memory.mismatch_detected());
    assert_eq!(status_b.continuity_generation, 1);
}
