//! Real rustls handshakes over in-memory transport; no external network.
#![cfg(feature = "host-linux")]
use rustls::{
    pki_types::{CertificateDer, PrivatePkcs8KeyDer, ServerName, UnixTime},
    ClientConfig, ClientConnection, RootCertStore, ServerConfig, ServerConnection,
};
use std::{io::Cursor, sync::Arc, time::Duration};

#[derive(Debug)]
struct FixtureTime;
impl rustls::time_provider::TimeProvider for FixtureTime {
    fn current_time(&self) -> Option<UnixTime> {
        Some(UnixTime::since_unix_epoch(Duration::from_secs(
            1_800_000_000,
        )))
    }
}
fn handshake(host: &'static str) -> Result<ClientConnection, rustls::Error> {
    let root = CertificateDer::from(include_bytes!("fixtures/root.der").to_vec());
    let leaf = CertificateDer::from(include_bytes!("fixtures/localhost.der").to_vec());
    let key = PrivatePkcs8KeyDer::from(include_bytes!("fixtures/localhost-key.der").to_vec());
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let server_config = ServerConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(vec![leaf, root.clone()], key.into())
        .unwrap();
    let mut roots = RootCertStore::empty();
    roots.add(root).unwrap();
    let config = ClientConfig::builder_with_details(provider, Arc::new(FixtureTime))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(roots)
        .with_no_client_auth();
    let mut client =
        ClientConnection::new(Arc::new(config), ServerName::try_from(host).unwrap()).unwrap();
    let mut server = ServerConnection::new(Arc::new(server_config)).unwrap();
    for _ in 0..8 {
        let mut wire = Vec::new();
        client.write_tls(&mut wire).unwrap();
        server.read_tls(&mut Cursor::new(wire)).unwrap();
        server.process_new_packets()?;
        let mut wire = Vec::new();
        server.write_tls(&mut wire).unwrap();
        client.read_tls(&mut Cursor::new(wire)).unwrap();
        client.process_new_packets()?;
        if !client.is_handshaking() && !server.is_handshaking() {
            return Ok(client);
        }
    }
    panic!("fixture handshake did not complete");
}

#[test]
fn certificate_metadata_from_verified_tls_session() {
    let client = handshake("localhost").unwrap();
    let peers = client.peer_certificates().unwrap();
    let info = sunlight_cert::peer_chain(peers.iter().map(|der| der.as_ref())).unwrap();
    assert_eq!(info.subject, "CN=localhost");
    assert_eq!(info.issuer, "CN=Rabbit Test Root");
    assert_eq!(info.key_bits, Some(256));
    assert_eq!(info.signature_algorithm.as_deref(), Some("ECDSA / SHA-256"));
    assert_eq!(info.chain.len(), 2);
    assert_eq!(info.chain[0].subject, "CN=Rabbit Test Root");
    assert!(info.is_valid);
}

#[test]
fn hostname_failure_never_produces_verified_metadata() {
    assert!(matches!(
        handshake("wrong.invalid"),
        Err(rustls::Error::InvalidCertificate(_))
    ));
}
