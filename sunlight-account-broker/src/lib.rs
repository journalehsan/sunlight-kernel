//! Bounded OAuth coordinator. No filesystem API, GUI, IPC identity assumptions,
//! or network implementation in the core. Provider configuration is trusted code.
#![no_std]
extern crate alloc;
#[cfg(feature = "host-runtime")]
extern crate std;
use alloc::{
    collections::BTreeMap,
    format,
    string::{String, ToString},
    vec::Vec,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

#[cfg(feature = "host-runtime")]
pub mod host;
#[cfg(any(feature = "host-runtime", feature = "native-tls"))]
pub mod transport;

pub mod installed_provider;
pub mod storage;

pub const MAX_SESSIONS: usize = 8;
pub const MAX_ACCOUNTS: usize = 16;
pub const SESSION_TTL: u64 = 120;
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Error {
    Configuration,
    Scope,
    Capacity,
    Entropy,
    Callback,
    State,
    Expired,
    Provider,
    MalformedToken,
    Timeout,
    Tls,
    Kv,
    DurableStorageUnavailable,
    Unauthorized,
    NotFound,
    Browser,
}
pub type Result<T> = core::result::Result<T, Error>;

/// Construct only from installed trusted configuration, never from app IPC.
#[derive(Clone)]
pub struct Provider {
    pub id: &'static str,
    pub authorization_endpoint: &'static str,
    pub token_endpoint: &'static str,
    pub client_id: &'static str,
    pub allowed_scopes: &'static [&'static str],
}
pub trait Entropy {
    fn fill(&mut self, bytes: &mut [u8]) -> Result<()>;
}
pub trait TokenTransport {
    /// Must POST once, with verified TLS, no redirects, a bounded body and timeout.
    fn exchange(&mut self, endpoint: &str, form: &str) -> Result<TokenResponse>;
}
pub struct TokenResponse {
    pub status: u16,
    pub body: Zeroizing<String>,
}
/// Explicit secret-store interface. Values and keys never go to the filesystem.
pub trait KvClient {
    fn put(&mut self, account: &str, secret: &str) -> Result<()>;
    fn get(&mut self, account: &str) -> Result<Zeroizing<String>>;
    fn delete(&mut self, account: &str) -> Result<()>;
}
/// Production gate until broker-exclusive native persistence is proven.
/// Namespace isolation alone does not prove commit, recovery or deletion.
pub struct UnavailableDurableKv;
impl KvClient for UnavailableDurableKv {
    fn put(&mut self, _: &str, _: &str) -> Result<()> {
        Err(Error::DurableStorageUnavailable)
    }
    fn get(&mut self, _: &str) -> Result<Zeroizing<String>> {
        Err(Error::DurableStorageUnavailable)
    }
    fn delete(&mut self, _: &str) -> Result<()> {
        Err(Error::DurableStorageUnavailable)
    }
}
/// Deterministic, bounded volatile backend. Never mapped onto production KV keys.
#[derive(Default)]
pub struct MemoryKv {
    values: BTreeMap<String, Zeroizing<String>>,
    pub writes: usize,
}
impl KvClient for MemoryKv {
    fn put(&mut self, key: &str, value: &str) -> Result<()> {
        if !self.values.contains_key(key) && self.values.len() >= MAX_ACCOUNTS {
            return Err(Error::Capacity);
        }
        self.values.insert(key.into(), Zeroizing::new(value.into()));
        self.writes += 1;
        Ok(())
    }
    fn get(&mut self, key: &str) -> Result<Zeroizing<String>> {
        self.values.get(key).cloned().ok_or(Error::NotFound)
    }
    fn delete(&mut self, key: &str) -> Result<()> {
        self.values.remove(key);
        Ok(())
    }
}

impl MemoryKv {
    /// Seeding helper for tests: store one value without counting a write.
    pub fn with_value(key: &str, value: &str) -> Self {
        let mut values = BTreeMap::new();
        values.insert(key.into(), Zeroizing::new(value.into()));
        Self { values, writes: 0 }
    }
}
pub trait AccessPolicy {
    fn allows(&self, caller: u64, account: &str, scopes: &[&str]) -> bool;
}
/// Production default: caller PID alone is not an authorization grant.
pub struct DenyAccess;
impl AccessPolicy for DenyAccess {
    fn allows(&self, _: u64, _: &str, _: &[&str]) -> bool {
        false
    }
}

struct Session {
    state: Zeroizing<String>,
    verifier: Zeroizing<String>,
    redirect: String,
    scopes: Vec<String>,
    expires: u64,
}
struct Account {
    scopes: Vec<String>,
    expires: u64,
}
/// The returned account handle is local and opaque; not a provider subject ID.
#[derive(Debug, PartialEq, Eq)]
pub struct AccountResult {
    pub account: String,
}
// Deliberately no Debug on secrets or objects containing them.
pub struct Started {
    pub authorization_url: String,
    pub session: String,
}
pub struct Access {
    pub bearer: Zeroizing<String>,
    pub expires_at: u64,
}
pub struct Broker<K, E, T, P = DenyAccess> {
    provider: Provider,
    pub kv: K,
    entropy: E,
    transport: T,
    policy: P,
    sessions: Vec<Session>,
    accounts: BTreeMap<String, Account>,
}
impl<K: KvClient, E: Entropy, T: TokenTransport, P: AccessPolicy> Broker<K, E, T, P> {
    pub fn new(provider: Provider, kv: K, entropy: E, transport: T, policy: P) -> Result<Self> {
        if provider.id.is_empty()
            || provider.client_id.is_empty()
            || !trusted_https(provider.authorization_endpoint)
            || !trusted_https(provider.token_endpoint)
        {
            return Err(Error::Configuration);
        }
        Ok(Self {
            provider,
            kv,
            entropy,
            transport,
            policy,
            sessions: Vec::new(),
            accounts: BTreeMap::new(),
        })
    }
    /// Loopback is the only implemented redirect method. The runtime must bind
    /// this exact ephemeral endpoint BEFORE calling this operation.
    pub fn start_authorization(
        &mut self,
        scopes: &[&str],
        redirect: &str,
        now: u64,
    ) -> Result<Started> {
        self.sessions.retain(|s| s.expires > now);
        if self.sessions.len() >= MAX_SESSIONS {
            return Err(Error::Capacity);
        }
        validate_redirect(redirect)?;
        if scopes.is_empty()
            || scopes.len() > 16
            || scopes
                .iter()
                .any(|s| !self.provider.allowed_scopes.contains(s))
        {
            return Err(Error::Scope);
        }
        let mut bytes = Zeroizing::new([0u8; 32]);
        self.entropy.fill(&mut bytes[..])?;
        let state = Zeroizing::new(URL_SAFE_NO_PAD.encode(&bytes[..]));
        self.entropy.fill(&mut bytes[..])?;
        let verifier = Zeroizing::new(URL_SAFE_NO_PAD.encode(&bytes[..]));
        if self.sessions.iter().any(|s| s.state == state) {
            return Err(Error::Entropy);
        }
        let challenge = s256(&verifier);
        let url = format!(
            "{}?{}",
            self.provider.authorization_endpoint,
            form(&[
                ("response_type", "code"),
                ("client_id", self.provider.client_id),
                ("redirect_uri", redirect),
                ("scope", &scopes.join(" ")),
                ("state", &state),
                ("code_challenge", &challenge),
                ("code_challenge_method", "S256"),
            ])
        );
        let handle = state.to_string();
        self.sessions.push(Session {
            state,
            verifier,
            redirect: redirect.into(),
            scopes: scopes.iter().map(|s| s.to_string()).collect(),
            expires: now.saturating_add(SESSION_TTL),
        });
        Ok(Started {
            authorization_url: url,
            session: handle,
        })
    }
    /// Drop expired secrets even when no further authorization is started.
    pub fn expire_sessions(&mut self, now: u64) {
        self.sessions.retain(|session| session.expires > now);
    }

    /// Cancellation consumes the session and zeroizes its verifier and state.
    pub fn cancel_authorization(&mut self, handle: &str) -> Result<()> {
        let index = self
            .sessions
            .iter()
            .position(|session| constant_eq(session.state.as_bytes(), handle.as_bytes()))
            .ok_or(Error::NotFound)?;
        self.sessions.remove(index);
        Ok(())
    }

    /// A callback transport must bind the callback to its own expected session,
    /// in addition to validating the redirect target and state in the core.
    pub fn complete_session(
        &mut self,
        expected: &str,
        callback: &str,
        now: u64,
    ) -> Result<AccountResult> {
        let (_, query) = callback.split_once('?').ok_or(Error::Callback)?;
        let fields = parse_form(query)?;
        let state = fields.get("state").ok_or(Error::State)?;
        if !constant_eq(expected.as_bytes(), state.as_bytes()) {
            return Err(Error::State);
        }
        self.complete_authorization(callback, now)
    }

    pub fn complete_authorization(&mut self, callback: &str, now: u64) -> Result<AccountResult> {
        if callback.len() > 8192 || callback.contains('#') {
            return Err(Error::Callback);
        }
        let (target, query) = callback.split_once('?').ok_or(Error::Callback)?;
        let fields = parse_form(query)?;
        let state = fields.get("state").ok_or(Error::State)?;
        let index = self
            .sessions
            .iter()
            .position(|s| constant_eq(s.state.as_bytes(), state.as_bytes()))
            .ok_or(Error::State)?;
        // Consume before any network/KV action, including failures.
        let session = self.sessions.remove(index);
        if now >= session.expires {
            return Err(Error::Expired);
        }
        if target != session.redirect {
            return Err(Error::Callback);
        }
        if fields.keys().any(|k| {
            !["state", "code", "error", "error_description", "error_uri"].contains(&k.as_str())
                && !(self.provider.id == "microsoft-consumers" && k == "session_state")
        }) {
            return Err(Error::Callback);
        }
        if fields.contains_key("error") {
            return if fields.contains_key("code") {
                Err(Error::Callback)
            } else {
                Err(Error::Provider)
            };
        }
        let code = fields
            .get("code")
            .filter(|c| !c.is_empty() && c.len() <= 2048)
            .ok_or(Error::Callback)?;
        let body = Zeroizing::new(form(&[
            ("grant_type", "authorization_code"),
            ("client_id", self.provider.client_id),
            ("redirect_uri", &session.redirect),
            ("code", code),
            ("code_verifier", &session.verifier),
        ]));
        let response = self
            .transport
            .exchange(self.provider.token_endpoint, &body)?;
        let token = parse_token(&response, &session.scopes)?;
        if self.accounts.len() >= MAX_ACCOUNTS {
            return Err(Error::Capacity);
        }
        let mut id = [0u8; 32];
        self.entropy.fill(&mut id)?;
        let account = format!("acct-{}", URL_SAFE_NO_PAD.encode(id));
        if self.accounts.contains_key(&account) {
            return Err(Error::Entropy);
        }
        self.kv.put(&account, &response.body)?;
        self.accounts.insert(
            account.clone(),
            Account {
                scopes: session.scopes,
                expires: now.saturating_add(token.expires_in),
            },
        );
        Ok(AccountResult { account })
    }
    pub fn request_access(
        &mut self,
        caller: u64,
        account: &str,
        scopes: &[&str],
        now: u64,
    ) -> Result<Access> {
        if !self.policy.allows(caller, account, scopes) {
            return Err(Error::Unauthorized);
        }
        let a = self.accounts.get(account).ok_or(Error::NotFound)?;
        if scopes.is_empty() || scopes.iter().any(|s| !a.scopes.iter().any(|v| v == s)) {
            return Err(Error::Scope);
        }
        if now >= a.expires {
            return Err(Error::Expired);
        }
        let raw = self.kv.get(account)?;
        let response = TokenResponse {
            status: 200,
            body: raw,
        };
        let mut token = parse_token(&response, &a.scopes)?;
        Ok(Access {
            bearer: Zeroizing::new(core::mem::take(&mut token.access_token)),
            expires_at: a.expires,
        })
    }
    pub fn disconnect(&mut self, account: &str) -> Result<()> {
        self.kv.delete(account)?;
        self.accounts.remove(account);
        Ok(())
    }
}
fn trusted_https(s: &str) -> bool {
    s.starts_with("https://")
        && s.len() <= 2048
        && !s[8..].is_empty()
        && !s.contains(['?', '#', '@'])
        && !s.bytes().any(|b| b <= 32 || b == 127)
}
fn validate_redirect(s: &str) -> Result<()> {
    let port = s
        .strip_prefix("http://127.0.0.1:")
        .and_then(|s| s.strip_suffix("/oauth/callback"))
        .and_then(|s| s.parse::<u16>().ok())
        .filter(|p| *p != 0);
    if port.is_none() {
        Err(Error::Callback)
    } else {
        Ok(())
    }
}
pub fn s256(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}
fn constant_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |v, (a, b)| v | (a ^ b)) == 0
}
pub fn form(fields: &[(&str, &str)]) -> String {
    fields
        .iter()
        .map(|(k, v)| format!("{}={}", encode(k), encode(v)))
        .collect::<Vec<_>>()
        .join("&")
}
fn encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}
/// Strict single decode. Reject ambiguous duplicate fields and invalid escapes.
pub fn parse_form(s: &str) -> Result<BTreeMap<String, String>> {
    if s.len() > 8192 {
        return Err(Error::Callback);
    }
    let mut out = BTreeMap::new();
    for field in s.split('&') {
        let (k, v) = field.split_once('=').ok_or(Error::Callback)?;
        let k = decode(k)?;
        let v = decode(v)?;
        if out.len() >= 16 || out.insert(k, v).is_some() {
            return Err(Error::Callback);
        }
    }
    Ok(out)
}
fn decode(s: &str) -> Result<String> {
    let mut out = Vec::new();
    let mut it = s.bytes();
    while let Some(b) = it.next() {
        out.push(match b {
            b'%' => {
                let a = (it.next().ok_or(Error::Callback)? as char)
                    .to_digit(16)
                    .ok_or(Error::Callback)?;
                let b = (it.next().ok_or(Error::Callback)? as char)
                    .to_digit(16)
                    .ok_or(Error::Callback)?;
                (a * 16 + b) as u8
            }
            b'+' => b' ',
            _ => b,
        });
    }
    if out.iter().any(|b| *b < 32 || *b == 127) {
        return Err(Error::Callback);
    }
    String::from_utf8(out).map_err(|_| Error::Callback)
}
#[derive(Deserialize)]
struct Token {
    access_token: String,
    token_type: String,
    expires_in: u64,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    refresh_token: Option<String>,
}
impl Drop for Token {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.access_token.zeroize();
        self.refresh_token.zeroize();
    }
}
fn parse_token(response: &TokenResponse, scopes: &[String]) -> Result<Token> {
    if response.body.len() > 16384 {
        return Err(Error::MalformedToken);
    }
    if !(200..300).contains(&response.status) {
        return Err(Error::Provider);
    }
    let value: Token = serde_json::from_str(&response.body).map_err(|_| Error::MalformedToken)?;
    if !value.token_type.eq_ignore_ascii_case("Bearer")
        || value.access_token.is_empty()
        || value.access_token.len() > 4096
        || value.access_token.bytes().any(|b| !(33..=126).contains(&b))
        || value.expires_in == 0
        || value.expires_in > 86400 * 365
        || value.scope.as_ref().is_some_and(|s| {
            let granted: Vec<_> = s.split_whitespace().collect();
            granted.len() != scopes.len() || scopes.iter().any(|s| !granted.contains(&s.as_str()))
        })
    {
        return Err(Error::MalformedToken);
    }
    Ok(value)
}
