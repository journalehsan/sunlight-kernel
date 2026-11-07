use sunlight_account_broker::*;
use zeroize::Zeroizing;
const REDIRECT: &str = "http://127.0.0.1:49152/oauth/callback";
fn provider() -> Provider {
    Provider {
        id: "mock",
        authorization_endpoint: "https://mock.invalid/authorize",
        token_endpoint: "https://mock.invalid/token",
        client_id: "public-native",
        allowed_scopes: &["mail.read", "files.read"],
    }
}
struct Counter(u8);
impl Entropy for Counter {
    fn fill(&mut self, bytes: &mut [u8]) -> Result<()> {
        self.0 += 1;
        bytes.fill(self.0);
        Ok(())
    }
}
struct MockProvider {
    response: &'static str,
    status: u16,
    failure: Option<Error>,
}
impl Default for MockProvider {
    fn default() -> Self {
        Self {
            status: 200,
            failure: None,
            response: r#"{"access_token":"access-only","refresh_token":"never-to-app","token_type":"Bearer","expires_in":3600,"scope":"mail.read"}"#,
        }
    }
}
impl TokenTransport for MockProvider {
    fn exchange(&mut self, endpoint: &str, body: &str) -> Result<TokenResponse> {
        assert_eq!(endpoint, "https://mock.invalid/token");
        let p = parse_form(body).unwrap();
        assert_eq!(p["grant_type"], "authorization_code");
        assert_eq!(p["client_id"], "public-native");
        assert!(!p.contains_key("client_secret"));
        assert_eq!(p["code_verifier"].len(), 43);
        if let Some(e) = self.failure {
            return Err(e);
        }
        Ok(TokenResponse {
            status: self.status,
            body: Zeroizing::new(self.response.into()),
        })
    }
}
struct TrustedHarness;
impl AccessPolicy for TrustedHarness {
    fn allows(&self, caller: u64, _: &str, _: &[&str]) -> bool {
        caller == 7
    }
}
type Harness = Broker<MemoryKv, Counter, MockProvider, TrustedHarness>;
fn broker() -> Harness {
    Broker::new(
        provider(),
        MemoryKv::default(),
        Counter(0),
        MockProvider::default(),
        TrustedHarness,
    )
    .unwrap()
}
fn callback(s: &Started) -> String {
    format!(
        "{REDIRECT}?{}",
        form(&[("state", &s.session), ("code", "one-use-code")])
    )
}
#[test]
fn s256_rfc_vector() {
    assert_eq!(
        s256("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
        "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
    );
}
#[test]
fn authorization_url_and_kv_access_disconnect() {
    let mut b = broker();
    let s = b.start_authorization(&["mail.read"], REDIRECT, 0).unwrap();
    let p = parse_form(s.authorization_url.split_once('?').unwrap().1).unwrap();
    assert_eq!(p["response_type"], "code");
    assert_eq!(p["scope"], "mail.read");
    assert_eq!(p["redirect_uri"], REDIRECT);
    assert_eq!(p["code_challenge_method"], "S256");
    assert_eq!(p["state"].len(), 43);
    assert_ne!(p["state"], p["code_challenge"]);
    let a = b.complete_authorization(&callback(&s), 1).unwrap();
    assert_eq!(b.kv.writes, 1);
    assert_eq!(
        b.request_access(8, &a.account, &["mail.read"], 2).err(),
        Some(Error::Unauthorized)
    );
    let access = b.request_access(7, &a.account, &["mail.read"], 2).unwrap();
    assert_eq!(&*access.bearer, "access-only");
    assert_eq!(
        b.request_access(7, &a.account, &["files.read"], 2).err(),
        Some(Error::Scope)
    );
    assert_eq!(
        b.request_access(7, &a.account, &["mail.read"], 3601).err(),
        Some(Error::Expired)
    );
    b.disconnect(&a.account).unwrap();
    assert_eq!(b.kv.get(&a.account).err(), Some(Error::NotFound));
}
#[test]
fn replay_wrong_state_expiry_and_target() {
    let mut b = broker();
    let s = b.start_authorization(&["mail.read"], REDIRECT, 0).unwrap();
    assert_eq!(
        b.complete_authorization(&format!("{REDIRECT}?state=wrong&code=x"), 1)
            .err(),
        Some(Error::State)
    );
    b.complete_authorization(&callback(&s), 1).unwrap();
    assert_eq!(
        b.complete_authorization(&callback(&s), 1).err(),
        Some(Error::State)
    );
    let s = b.start_authorization(&["mail.read"], REDIRECT, 0).unwrap();
    assert_eq!(
        b.complete_authorization(&callback(&s), SESSION_TTL).err(),
        Some(Error::Expired)
    );
    let s = b.start_authorization(&["mail.read"], REDIRECT, 0).unwrap();
    assert_eq!(
        b.complete_authorization(&callback(&s).replace("/oauth/callback", "/wrong"), 1)
            .err(),
        Some(Error::Callback)
    );
    assert_eq!(
        b.complete_authorization(&callback(&s), 1).err(),
        Some(Error::State)
    );
}
#[test]
fn malformed_callbacks_provider_denial_and_session_capacity() {
    let mut b = broker();
    for query in [
        "state=x&state=y",
        "state=%GG",
        "state=%00",
        "code=x",
        "state=x#code=x",
    ] {
        assert!(b
            .complete_authorization(&format!("{REDIRECT}?{query}"), 0)
            .is_err());
    }
    let s = b.start_authorization(&["mail.read"], REDIRECT, 0).unwrap();
    let callback = format!(
        "{REDIRECT}?state={}&error=access_denied&error_description=secret",
        s.session
    );
    assert_eq!(
        b.complete_authorization(&callback, 1).err(),
        Some(Error::Provider)
    );
    assert_eq!(b.kv.writes, 0);
    for _ in 0..MAX_SESSIONS {
        b.start_authorization(&["mail.read"], REDIRECT, 0).unwrap();
    }
    assert_eq!(
        b.start_authorization(&["mail.read"], REDIRECT, 0).err(),
        Some(Error::Capacity)
    );
    b.start_authorization(&["mail.read"], REDIRECT, SESSION_TTL)
        .unwrap();
    assert_eq!(
        b.start_authorization(&["evil"], REDIRECT, SESSION_TTL)
            .err(),
        Some(Error::Scope)
    );
    assert_eq!(
        b.start_authorization(&["mail.read"], "http://evil/cb", SESSION_TTL)
            .err(),
        Some(Error::Callback)
    );
}
#[test]
fn exchange_failures_consume_session_without_kv_writes() {
    for error in [Error::Timeout, Error::Tls] {
        let transport = MockProvider {
            failure: Some(error),
            ..MockProvider::default()
        };
        let mut b = Broker::new(
            provider(),
            MemoryKv::default(),
            Counter(0),
            transport,
            DenyAccess,
        )
        .unwrap();
        let s = b.start_authorization(&["mail.read"], REDIRECT, 0).unwrap();
        assert_eq!(
            b.complete_authorization(&callback(&s), 1).err(),
            Some(error)
        );
        assert_eq!(
            b.complete_authorization(&callback(&s), 1).err(),
            Some(Error::State)
        );
        assert_eq!(b.kv.writes, 0);
    }
}
#[test]
fn token_provider_errors_and_malformed_tokens() {
    for (status, response, error) in [
        (400, r#"{"error":"invalid_grant"}"#, Error::Provider),
        (200, "not json", Error::MalformedToken),
        (
            200,
            r#"{"access_token":"a","token_type":"Bearer","expires_in":0}"#,
            Error::MalformedToken,
        ),
        (
            200,
            r#"{"access_token":"a","token_type":"Bearer","expires_in":30,"scope":"files.read"}"#,
            Error::MalformedToken,
        ),
        (307, "", Error::Provider),
    ] {
        let mut b = Broker::new(
            provider(),
            MemoryKv::default(),
            Counter(0),
            MockProvider {
                status,
                response,
                failure: None,
            },
            DenyAccess,
        )
        .unwrap();
        let s = b.start_authorization(&["mail.read"], REDIRECT, 0).unwrap();
        assert_eq!(
            b.complete_authorization(&callback(&s), 1).err(),
            Some(error)
        );
        assert_eq!(b.kv.writes, 0);
    }
}
struct FailingKv;
impl KvClient for FailingKv {
    fn put(&mut self, _: &str, _: &str) -> Result<()> {
        Err(Error::Kv)
    }
    fn get(&mut self, _: &str) -> Result<Zeroizing<String>> {
        Err(Error::Kv)
    }
    fn delete(&mut self, _: &str) -> Result<()> {
        Err(Error::Kv)
    }
}
fn fails_closed(kv: impl KvClient, expected: Error) {
    let mut b = Broker::new(
        provider(),
        kv,
        Counter(0),
        MockProvider::default(),
        DenyAccess,
    )
    .unwrap();
    let s = b.start_authorization(&["mail.read"], REDIRECT, 0).unwrap();
    assert_eq!(
        b.complete_authorization(&callback(&s), 1).err(),
        Some(expected)
    );
    assert_eq!(
        b.request_access(0, "any", &["mail.read"], 1).err(),
        Some(Error::Unauthorized)
    );
}
#[test]
fn kv_failure_and_unavailable_durable_storage() {
    fails_closed(FailingKv, Error::Kv);
    fails_closed(UnavailableDurableKv, Error::DurableStorageUnavailable);
}
#[test]
fn production_access_is_disabled_even_for_uid_zero_or_claimed_pid() {
    let mut b = Broker::new(
        provider(),
        MemoryKv::default(),
        Counter(0),
        MockProvider::default(),
        DenyAccess,
    )
    .unwrap();
    let s = b.start_authorization(&["mail.read"], REDIRECT, 0).unwrap();
    let account = b.complete_authorization(&callback(&s), 1).unwrap().account;
    for caller in [0, 1, 7, u64::MAX] {
        assert_eq!(
            b.request_access(caller, &account, &["mail.read"], 2).err(),
            Some(Error::Unauthorized)
        );
    }
}
#[cfg(feature = "host-runtime")]
mod local {
    use super::*;
    use std::{
        io::{Read, Write},
        net::{TcpListener, TcpStream},
        sync::{Arc, Mutex},
        thread,
        time::Duration,
    };
    use sunlight_account_broker::host::{Loopback, OsEntropy};
    // HTTP is intentionally confined to this test fixture. Production transport
    // is VerifiedTls, which refuses HTTP and never follows redirects.
    struct LocalProvider {
        port: u16,
        challenge: Arc<Mutex<String>>,
    }
    impl TokenTransport for LocalProvider {
        fn exchange(&mut self, endpoint: &str, body: &str) -> Result<TokenResponse> {
            assert_eq!(endpoint, "https://mock.invalid/token");
            let mut stream = TcpStream::connect(("127.0.0.1", self.port)).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            write!(
                stream,
                "POST /token HTTP/1.1\r\nHost: mock.invalid\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            )
            .unwrap();
            let mut response = String::new();
            stream.read_to_string(&mut response).unwrap();
            let body = response.split_once("\r\n\r\n").unwrap().1.to_string();
            assert!(!self.challenge.lock().unwrap().is_empty());
            Ok(TokenResponse {
                status: 200,
                body: Zeroizing::new(body),
            })
        }
    }
    #[test]
    fn local_provider_end_to_end_with_real_callback_socket() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let challenge = Arc::new(Mutex::new(String::new()));
        let expected = challenge.clone();
        let provider_thread = thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            s.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
            let mut buf = Vec::new();
            loop {
                let mut chunk = [0; 1024];
                let n = s.read(&mut chunk).unwrap();
                assert!(n > 0);
                buf.extend_from_slice(&chunk[..n]);
                if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    let head = std::str::from_utf8(&buf[..i]).unwrap();
                    let len: usize = head
                        .lines()
                        .find_map(|s| s.strip_prefix("Content-Length: "))
                        .unwrap()
                        .parse()
                        .unwrap();
                    if buf.len() >= i + 4 + len {
                        break;
                    }
                }
            }
            let raw = String::from_utf8(buf).unwrap();
            assert!(raw.starts_with("POST /token HTTP/1.1"));
            let params = parse_form(raw.split_once("\r\n\r\n").unwrap().1).unwrap();
            assert_eq!(params["code"], "single-use-code");
            assert_eq!(s256(&params["code_verifier"]), *expected.lock().unwrap());
            assert_eq!(params["grant_type"], "authorization_code");
            let body = MockProvider::default().response;
            write!(
                s,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .unwrap();
        });
        let transport = LocalProvider {
            port,
            challenge: challenge.clone(),
        };
        let mut b = Broker::new(
            provider(),
            MemoryKv::default(),
            OsEntropy,
            transport,
            TrustedHarness,
        )
        .unwrap();
        let callback = Loopback::bind().unwrap();
        let mut browser = None;
        callback.start(&mut b, &["mail.read"], 0, |url| {
            let p = parse_form(url.split_once('?').unwrap().1).unwrap();
            *challenge.lock().unwrap() = p["code_challenge"].clone();
            let redirect = p["redirect_uri"].clone(); let state = p["state"].clone();
            // A deterministic mock browser/provider redirect, never provider JS.
            browser = Some(thread::spawn(move || {
                let authority = redirect.strip_prefix("http://").unwrap().split('/').next().unwrap();
                let mut stream = TcpStream::connect(authority).unwrap();
                stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
                write!(stream, "GET /oauth/callback?state={state}&code=single-use-code HTTP/1.1\r\nHost: {authority}\r\n\r\n").unwrap();
                let mut response = String::new(); stream.read_to_string(&mut response).unwrap();
                assert!(response.contains("Location: about:blank"));
                assert!(!response.contains("single-use-code")); assert!(!response.contains("access-only"));
            })); Ok(())
        }).unwrap();
        let account = callback
            .complete(&mut b, 0, Duration::from_secs(3))
            .unwrap()
            .account;
        browser.unwrap().join().unwrap();
        provider_thread.join().unwrap();
        assert_eq!(b.kv.writes, 1);
        assert_eq!(
            b.request_access(99, &account, &["mail.read"], 1).err(),
            Some(Error::Unauthorized)
        );
        assert_eq!(
            &*b.request_access(7, &account, &["mail.read"], 1)
                .unwrap()
                .bearer,
            "access-only"
        );
    }
    #[test]
    fn real_listener_timeout_and_fresh_entropy() {
        let mut b = Broker::new(
            provider(),
            MemoryKv::default(),
            OsEntropy,
            MockProvider::default(),
            DenyAccess,
        )
        .unwrap();
        let s = b.start_authorization(&["mail.read"], REDIRECT, 0).unwrap();
        let t = b.start_authorization(&["mail.read"], REDIRECT, 0).unwrap();
        assert_ne!(s.session, t.session);
        assert_eq!(
            Loopback::bind()
                .unwrap()
                .complete(&mut b, 0, Duration::from_millis(5))
                .err(),
            Some(Error::Timeout)
        );
    }
    #[test]
    fn verified_transport_rejects_plaintext() {
        assert_eq!(
            sunlight_account_broker::transport::VerifiedTls
                .exchange("http://127.0.0.1/token", "code=secret")
                .err(),
            Some(Error::Configuration)
        );
    }
}

#[cfg(feature = "host-runtime")]
#[test]
fn verified_transport_real_tls_failure_never_posts_code_in_plaintext() {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
        time::Duration,
    };
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut client_hello = [0u8; 4096];
        let n = stream.read(&mut client_hello).unwrap();
        assert!(!client_hello[..n]
            .windows(b"private-code".len())
            .any(|v| v == b"private-code"));
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
            .unwrap();
    });
    let result = sunlight_account_broker::transport::VerifiedTls.exchange(
        &format!("https://127.0.0.1:{port}/token"),
        "code=private-code&code_verifier=private-verifier",
    );
    assert_eq!(result.err(), Some(Error::Tls));
    server.join().unwrap();
}

#[test]
fn cancellation_consumes_only_the_selected_session() {
    let mut b = broker();
    let first = b.start_authorization(&["mail.read"], REDIRECT, 0).unwrap();
    let second = b.start_authorization(&["mail.read"], REDIRECT, 0).unwrap();
    b.cancel_authorization(&first.session).unwrap();
    assert_eq!(b.cancel_authorization(&first.session), Err(Error::NotFound));
    assert_eq!(
        b.complete_authorization(&callback(&first), 1),
        Err(Error::State)
    );
    b.complete_session(&second.session, &callback(&second), 1)
        .unwrap();
    assert_eq!(b.kv.writes, 1);
}

#[test]
fn transport_cannot_complete_a_different_live_session() {
    let mut b = broker();
    let first = b.start_authorization(&["mail.read"], REDIRECT, 0).unwrap();
    let second = b.start_authorization(&["mail.read"], REDIRECT, 0).unwrap();
    assert_eq!(
        b.complete_session(&first.session, &callback(&second), 1),
        Err(Error::State)
    );
    assert_eq!(b.kv.writes, 0);
    b.complete_session(&second.session, &callback(&second), 1)
        .unwrap();
    b.complete_session(&first.session, &callback(&first), 1)
        .unwrap();
    assert_eq!(b.kv.writes, 2);
}

#[test]
fn idle_expiry_reclaims_capacity_and_rejects_old_callbacks() {
    let mut b = broker();
    let expired = b.start_authorization(&["mail.read"], REDIRECT, 0).unwrap();
    b.expire_sessions(SESSION_TTL);
    assert_eq!(
        b.complete_authorization(&callback(&expired), SESSION_TTL),
        Err(Error::State)
    );
    assert_eq!(
        b.cancel_authorization(&expired.session),
        Err(Error::NotFound)
    );
    for _ in 0..MAX_SESSIONS {
        b.start_authorization(&["mail.read"], REDIRECT, SESSION_TTL)
            .unwrap();
    }
    assert_eq!(
        b.start_authorization(&["mail.read"], REDIRECT, SESSION_TTL)
            .err(),
        Some(Error::Capacity)
    );
}

#[cfg(feature = "host-runtime")]
#[test]
fn failed_browser_launch_reclaims_session_capacity() {
    use sunlight_account_broker::host::Loopback;
    let mut b = broker();
    let listener = Loopback::bind().unwrap();
    for _ in 0..MAX_SESSIONS * 2 {
        assert_eq!(
            listener
                .start(&mut b, &["mail.read"], 0, |_| Err(Error::Browser))
                .err(),
            Some(Error::Browser)
        );
    }
    assert_eq!(b.kv.writes, 0);
}

#[test]
fn microsoft_callback_metadata_is_provider_specific() {
    // Microsoft adds session_state to a code callback. It is metadata, never
    // identity proof, and must not broaden callback parsing for other providers.
    let mut p = provider();
    p.id = "microsoft-consumers";
    let mut b = Broker::new(
        p,
        MemoryKv::default(),
        Counter(0),
        MockProvider::default(),
        DenyAccess,
    )
    .unwrap();
    let s = b.start_authorization(&["mail.read"], REDIRECT, 0).unwrap();
    let account = b
        .complete_session(
            &s.session,
            &format!("{}&session_state=opaque-metadata", callback(&s)),
            1,
        )
        .unwrap();
    assert!(matches!(
        b.request_access(0, &account.account, &["mail.read"], 2),
        Err(Error::Unauthorized)
    ));
    b.disconnect(&account.account).unwrap();
    assert!(matches!(b.kv.get(&account.account), Err(Error::NotFound)));

    let mut b = broker();
    let s = b.start_authorization(&["mail.read"], REDIRECT, 0).unwrap();
    assert_eq!(
        b.complete_session(
            &s.session,
            &format!("{}&session_state=opaque-metadata", callback(&s)),
            1
        ),
        Err(Error::Callback)
    );
}
