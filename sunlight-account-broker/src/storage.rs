//! Live-session record layout for the broker-exclusive `account-broker:*` KV
//! namespace.
//!
//! Two record kinds live under one prefix:
//!
//! * `account-broker:index` — newline-separated account handles. It exists only
//!   because `sunlight-kv` returns at most three keys per `SCAN` and has no
//!   cursor, so the namespace cannot be enumerated directly.
//! * `account-broker:acct-<43>` — one JSON record per account, holding provider
//!   display metadata plus the provider tokens.
//!
//! The record is bounded by the KV value page (4096 bytes; keys use a
//! separate page in SHM2), and metadata never contains token material so
//! `ListAccounts` cannot leak a secret.

use crate::{Error, KvClient, Result};
use alloc::{
    format,
    string::{String, ToString},
    vec::Vec,
};
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, Zeroizing};

/// Upper bound on stored accounts; matches the previous in-memory bound.
pub const MAX_ACCOUNTS: usize = 16;
/// Index record key.
pub const INDEX_KEY: &str = "account-broker:index";
/// Handle prefix inside the index and in record keys.
pub const HANDLE_PREFIX: &str = "acct-";
/// `acct-` plus 32 random bytes as unpadded base64url.
pub const HANDLE_LEN: usize = 48;
/// Largest value accepted by KV SHM2. Key, value, ACL and log framing do not
/// share a transport page. This is an IPC bound, not a log-size limit.
pub const MAX_RECORD_BYTES: usize = 4096;
/// Longest display identifier retained (an email or provider login name).
///
/// Bounded so a full `ListAccounts` page of [`MAX_ACCOUNTS`] entries always
/// fits one 4 KiB KV/SHM page: 16 × (48 handle + 64 identifier + 20 fixed
/// fields + separators) < 4096.
pub const MAX_IDENTIFIER_BYTES: usize = 64;

/// Sign-in status shown by the control panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccountStatus {
    /// Tokens are present and not past their expiry.
    Connected,
    /// Tokens are present but expired; the account must be re-authorized.
    Expired,
}

impl AccountStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Connected => "connected",
            Self::Expired => "expired",
        }
    }
}

/// Token-free account description returned to the control panel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountMetadata {
    /// Opaque handle; never a provider subject identifier.
    pub account: String,
    /// Trusted provider id, e.g. `microsoft-consumers`.
    pub provider: String,
    /// Lower-detail login name (email) when the provider reported one.
    pub identifier: String,
    /// Current sign-in status.
    pub status: AccountStatus,
    /// Scopes granted at authorization time.
    pub scopes: Vec<String>,
    /// Expiry in the coordinator clock domain (native: seconds since boot).
    pub expires_at: u64,
}

/// Stored record. Tokens are zeroized when the record is dropped.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    v: u8,
    provider: String,
    identifier: String,
    scopes: Vec<String>,
    expires_at: u64,
    access_token: String,
    token_type: String,
    #[serde(default)]
    refresh_token: Option<String>,
}

impl Drop for Record {
    fn drop(&mut self) {
        self.access_token.zeroize();
        if let Some(token) = self.refresh_token.as_mut() {
            token.zeroize();
        }
    }
}

/// Token material read back for `RequestAccess`.
pub struct StoredTokens {
    pub access_token: Zeroizing<String>,
    pub expires_at: u64,
    pub scope: Vec<String>,
    pub provider: String,
}

/// True when `handle` is a well-formed opaque account handle.
pub fn is_valid_handle(handle: &str) -> bool {
    handle.len() == HANDLE_LEN
        && handle.starts_with(HANDLE_PREFIX)
        && handle[HANDLE_PREFIX.len()..]
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// True when `value` is acceptable as a display identifier.
pub fn is_valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_IDENTIFIER_BYTES
        && value.bytes().all(|b| (32..=126).contains(&b))
        && !value.trim().is_empty()
}

/// KV key for an account handle.
pub fn record_key(handle: &str) -> String {
    format!("account-broker:{handle}")
}

fn validate_scope_set(scopes: &[String]) -> Result<()> {
    if scopes.is_empty() || scopes.len() > 16 {
        return Err(Error::Scope);
    }
    if scopes
        .iter()
        .any(|s| s.is_empty() || s.len() > 128 || s.bytes().any(|b| b <= 32 || b == 127))
    {
        return Err(Error::Scope);
    }
    Ok(())
}

/// Serialize one account record, enforcing the page budget.
pub fn encode_record(
    provider: &str,
    identifier: &str,
    scopes: &[String],
    expires_at: u64,
    access_token: &str,
    token_type: &str,
    refresh_token: Option<&str>,
) -> Result<Zeroizing<String>> {
    validate_scope_set(scopes)?;
    if provider.is_empty() || provider.len() > 64 {
        return Err(Error::Configuration);
    }
    let identifier = if is_valid_identifier(identifier) {
        identifier.to_string()
    } else {
        String::new()
    };
    let record = Record {
        v: 1,
        provider: provider.to_string(),
        identifier,
        scopes: scopes.to_vec(),
        expires_at,
        access_token: access_token.to_string(),
        token_type: token_type.to_string(),
        refresh_token: refresh_token.map(|t| t.to_string()),
    };
    let encoded = Zeroizing::new(serde_json::to_string(&record).map_err(|_| Error::Kv)?);
    if encoded.len() > MAX_RECORD_BYTES {
        return Err(Error::Capacity);
    }
    Ok(encoded)
}

/// Parse and validate a stored record against the expected provider and scopes.
fn decode_record(raw: &str, provider: &str) -> Result<Record> {
    if raw.len() > MAX_RECORD_BYTES {
        return Err(Error::MalformedToken);
    }
    let record: Record = serde_json::from_str(raw).map_err(|_| Error::MalformedToken)?;
    if record.v != 1
        || record.provider != provider
        || !record.token_type.eq_ignore_ascii_case("Bearer")
        || record.access_token.is_empty()
        || record.access_token.len() > 4096
        || record
            .access_token
            .bytes()
            .any(|b| !(33..=126).contains(&b))
        || record.expires_at == 0
        || validate_scope_set(&record.scopes).is_err()
        || (!record.identifier.is_empty() && !is_valid_identifier(&record.identifier))
    {
        return Err(Error::MalformedToken);
    }
    if let Some(refresh) = record.refresh_token.as_ref() {
        if refresh.is_empty()
            || refresh.len() > 4096
            || refresh.bytes().any(|b| !(33..=126).contains(&b))
        {
            return Err(Error::MalformedToken);
        }
    }
    Ok(record)
}

/// Read an account record's metadata.
pub fn read_metadata(
    kv: &mut impl KvClient,
    provider: &str,
    handle: &str,
    now: u64,
) -> Result<AccountMetadata> {
    if !is_valid_handle(handle) {
        return Err(Error::NotFound);
    }
    let raw = kv.get(&record_key(handle))?;
    metadata_from(handle, provider, &raw, now)
}

fn metadata_from(handle: &str, provider: &str, raw: &str, now: u64) -> Result<AccountMetadata> {
    let record = decode_record(raw, provider)?;
    Ok(AccountMetadata {
        account: handle.to_string(),
        provider: record.provider.clone(),
        identifier: record.identifier.clone(),
        status: if now >= record.expires_at {
            AccountStatus::Expired
        } else {
            AccountStatus::Connected
        },
        scopes: record.scopes.clone(),
        expires_at: record.expires_at,
    })
}

/// Read token material for an authorized access request.
pub fn read_tokens(kv: &mut impl KvClient, provider: &str, handle: &str) -> Result<StoredTokens> {
    if !is_valid_handle(handle) {
        return Err(Error::NotFound);
    }
    let raw = kv.get(&record_key(handle))?;
    let mut record = decode_record(&raw, provider)?;
    Ok(StoredTokens {
        access_token: Zeroizing::new(core::mem::take(&mut record.access_token)),
        expires_at: record.expires_at,
        scope: record.scopes.clone(),
        provider: record.provider.clone(),
    })
}

/// Confirm a handle's record is really gone. `delete` alone is not evidence:
/// a store that ignores the delete (or an ACL denial that also reports
/// "missing") must not be presented as a completed disconnect.
pub fn verify_absent(kv: &mut impl KvClient, handle: &str) -> Result<()> {
    if !is_valid_handle(handle) {
        return Err(Error::NotFound);
    }
    match kv.get(&record_key(handle)) {
        Err(Error::NotFound) => Ok(()),
        Ok(_) => Err(Error::Kv),
        Err(error) => Err(error),
    }
}

/// Parse the index record into handles, rejecting anything malformed.
pub fn decode_index(raw: &str) -> Result<Vec<String>> {
    if raw.len() > MAX_ACCOUNTS * (HANDLE_LEN + 1) {
        return Err(Error::Kv);
    }
    let mut handles = Vec::new();
    for line in raw.split('\n') {
        if line.is_empty() {
            continue;
        }
        if !is_valid_handle(line) || handles.len() >= MAX_ACCOUNTS {
            return Err(Error::Kv);
        }
        if handles.iter().any(|existing| existing == line) {
            return Err(Error::Kv);
        }
        handles.push(line.to_string());
    }
    Ok(handles)
}

fn encode_index(handles: &[String]) -> Zeroizing<String> {
    let mut out = Zeroizing::new(String::new());
    for handle in handles {
        out.push_str(handle);
        out.push('\n');
    }
    out
}

/// Read the index. A missing index means "no accounts yet", not an error.
pub fn read_index(kv: &mut impl KvClient) -> Result<Vec<String>> {
    match kv.get(INDEX_KEY) {
        Ok(raw) => decode_index(&raw),
        Err(Error::NotFound) => Ok(Vec::new()),
        Err(error) => Err(error),
    }
}

/// Persist the index after a change.
pub fn write_index(kv: &mut impl KvClient, handles: &[String]) -> Result<()> {
    if handles.len() > MAX_ACCOUNTS {
        return Err(Error::Capacity);
    }
    let encoded = encode_index(handles);
    kv.put(INDEX_KEY, &encoded)
}

/// Append `handle` to the index, refusing capacity overflow and duplicates.
pub fn index_add(kv: &mut impl KvClient, handle: &str) -> Result<()> {
    if !is_valid_handle(handle) {
        return Err(Error::Kv);
    }
    let mut handles = read_index(kv)?;
    if handles.iter().any(|existing| existing == handle) {
        return Err(Error::Entropy);
    }
    if handles.len() >= MAX_ACCOUNTS {
        return Err(Error::Capacity);
    }
    handles.push(handle.to_string());
    write_index(kv, &handles)
}

/// Remove `handle` from the index and verify it is gone.
pub fn index_remove(kv: &mut impl KvClient, handle: &str) -> Result<()> {
    let mut handles = read_index(kv)?;
    let before = handles.len();
    handles.retain(|existing| existing != handle);
    if handles.len() == before {
        return Err(Error::NotFound);
    }
    write_index(kv, &handles)?;
    let verify = read_index(kv)?;
    if verify.iter().any(|existing| existing == handle) {
        return Err(Error::Kv);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MemoryKv;

    const HANDLE: &str = "acct-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

    fn encoded() -> Zeroizing<String> {
        encode_record(
            "mock",
            "user@example.test",
            &["mail.read".to_string()],
            1_700_000_000,
            "access-only",
            "Bearer",
            Some("refresh-only"),
        )
        .unwrap()
    }

    #[test]
    fn deletion_verification_never_conflates_denial_with_missing() {
        struct RefusingStore;
        impl KvClient for RefusingStore {
            fn put(&mut self, _: &str, _: &str) -> Result<()> {
                Err(Error::Kv)
            }
            fn get(&mut self, _: &str) -> Result<Zeroizing<String>> {
                Err(Error::Unauthorized)
            }
            fn delete(&mut self, _: &str) -> Result<()> {
                Ok(())
            }
        }
        let mut store = RefusingStore;
        store.delete(&record_key(HANDLE)).unwrap();
        assert_eq!(verify_absent(&mut store, HANDLE), Err(Error::Unauthorized));
        assert_eq!(read_index(&mut store), Err(Error::Unauthorized));
        let mut unavailable = crate::UnavailableDurableKv;
        assert_eq!(
            verify_absent(&mut unavailable, HANDLE),
            Err(Error::DurableStorageUnavailable)
        );
        assert_eq!(
            read_index(&mut unavailable),
            Err(Error::DurableStorageUnavailable)
        );
    }

    #[test]
    fn record_round_trips_without_exposing_tokens() {
        let raw = encoded();
        assert!(raw.len() < MAX_RECORD_BYTES);
        let meta = metadata_from(HANDLE, "mock", &raw, 1_700_000_000 - 1).unwrap();
        assert_eq!(meta.identifier, "user@example.test");
        assert_eq!(meta.status, AccountStatus::Connected);
        assert_eq!(meta.expires_at, 1_700_000_000);
        let rendered = alloc::format!("{meta:?}");
        assert!(!rendered.contains("access-only"));
        assert!(!rendered.contains("refresh-only"));

        let tokens = read_tokens(
            &mut MemoryKv::with_value(&record_key(HANDLE), &raw),
            "mock",
            HANDLE,
        )
        .unwrap();
        assert_eq!(&*tokens.access_token, "access-only");
        assert_eq!(tokens.scope, alloc::vec!["mail.read".to_string()]);
        assert_eq!(verify_absent(&mut MemoryKv::default(), HANDLE), Ok(()));
        assert_eq!(
            verify_absent(&mut MemoryKv::with_value(&record_key(HANDLE), &raw), HANDLE),
            Err(Error::Kv)
        );
    }

    #[test]
    fn record_rejects_wrong_provider_and_unknown_fields() {
        let raw = encoded();
        assert_eq!(
            metadata_from(HANDLE, "microsoft-consumers", &raw, 0).err(),
            Some(Error::MalformedToken)
        );
        let tampered = raw.replace("\"v\":1", "\"v\":1,\"extra\":0");
        assert_eq!(
            metadata_from(HANDLE, "mock", &tampered, 0).err(),
            Some(Error::MalformedToken)
        );
        let expired = metadata_from(HANDLE, "mock", &raw, 1_700_000_000).unwrap();
        assert_eq!(expired.status, AccountStatus::Expired);
    }

    #[test]
    fn realistic_token_metadata_records_use_the_entire_value_page() {
        let scopes = ["mail.read".to_string()];
        let base = encode_record(
            "mock",
            "user@example.test",
            &scopes,
            3601,
            "a",
            "Bearer",
            Some("refresh"),
        )
        .unwrap()
        .len();
        for size in [1024, 2048, 4096] {
            let access = "a".repeat(size - base + 1);
            let raw = encode_record(
                "mock",
                "user@example.test",
                &scopes,
                3601,
                &access,
                "Bearer",
                Some("refresh"),
            )
            .unwrap();
            assert_eq!(raw.len(), size);
            let mut kv = MemoryKv::with_value(&record_key(HANDLE), &raw);
            let metadata = read_metadata(&mut kv, "mock", HANDLE, 1).unwrap();
            assert_eq!(metadata.status, AccountStatus::Connected);
            assert_eq!(metadata.identifier, "user@example.test");
            let tokens = read_tokens(&mut kv, "mock", HANDLE).unwrap();
            assert_eq!(&*tokens.access_token, &access);
        }
    }

    #[test]
    fn oversized_records_are_refused_before_the_kv_page_limit() {
        let token = "a".repeat(4096);
        assert_eq!(
            encode_record(
                "mock",
                "",
                &["mail.read".to_string()],
                1,
                &token,
                "Bearer",
                None
            )
            .err(),
            Some(Error::Capacity)
        );
    }

    #[test]
    fn index_round_trips_and_rejects_foreign_entries() {
        let handles = alloc::vec![HANDLE.to_string()];
        let encoded = encode_index(&handles);
        assert_eq!(decode_index(&encoded).unwrap(), handles);
        assert!(decode_index("").unwrap().is_empty());
        for bad in [
            "account-broker:x\n",
            "acct-short\n",
            &alloc::format!("{}\n{}", HANDLE, HANDLE),
        ] {
            assert!(decode_index(bad).is_err(), "{bad:?} must be rejected");
        }
        assert!(is_valid_handle(HANDLE));
        assert!(!is_valid_handle("acct-"));
        assert_eq!(HANDLE.len(), HANDLE_LEN);
        assert_eq!(HANDLE.len() - HANDLE_PREFIX.len(), 43);
    }
}
