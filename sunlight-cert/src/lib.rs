//! Bounded certificate display metadata. This module never verifies certificates.
#![no_std]
extern crate alloc;
use alloc::{format, string::String, vec::Vec};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CertificateTrust {
    Unverified,
    SessionSupplied,
    TrustAnchor,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertInfo {
    pub verified_host: Option<String>,
    pub subject: String,
    pub issuer: String,
    pub valid_from: Option<String>,
    pub valid_to: Option<String>,
    pub signature_algorithm: Option<String>,
    pub key_bits: Option<u32>,
    /// Available peer chain, root-most first. Entries have empty chains.
    pub chain: Vec<CertInfo>,
    pub is_valid: bool,
    pub trust: CertificateTrust,
    pub validation_errors: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ConnectionSecurity {
    #[default]
    NotSecure,
    Secure,
    Invalid,
    Unknown,
}

// Small DER reader: definite, minimally encoded lengths only; no unchecked indexing.
fn item<'a>(input: &mut &'a [u8]) -> Option<(u8, &'a [u8])> {
    let (&tag, rest) = input.split_first()?;
    let (&first, mut rest) = rest.split_first()?;
    let mut len = usize::from(first);
    if first & 128 != 0 {
        let count = usize::from(first & 127);
        if count == 0 || count > 4 {
            return None;
        }
        let bytes = rest.get(..count)?;
        if bytes[0] == 0 {
            return None;
        }
        len = 0;
        for b in bytes {
            len = len.checked_mul(256)?.checked_add(usize::from(*b))?;
        }
        if len < 128 {
            return None;
        }
        rest = rest.get(count..)?;
    }
    let value = rest.get(..len)?;
    *input = rest.get(len..)?;
    Some((tag, value))
}
fn take<'a>(input: &mut &'a [u8], tag: u8) -> Option<&'a [u8]> {
    let (actual, value) = item(input)?;
    (actual == tag).then_some(value)
}
fn name(mut input: &[u8]) -> Option<String> {
    let mut result = String::new();
    while !input.is_empty() {
        let mut set = take(&mut input, 0x31)?;
        while !set.is_empty() {
            let mut pair = take(&mut set, 0x30)?;
            let oid = take(&mut pair, 6)?;
            let (tag, value) = item(&mut pair)?;
            let label = match oid {
                [85, 4, 3] => "CN",
                [85, 4, 10] => "O",
                [85, 4, 11] => "OU",
                [85, 4, 6] => "C",
                _ => continue,
            };
            if !matches!(tag, 12 | 19 | 20 | 22) {
                continue;
            }
            let text = core::str::from_utf8(value).ok()?;
            if !result.is_empty() {
                result.push_str(", ");
            }
            result.push_str(label);
            result.push('=');
            result.extend(text.chars().map(|c| if c.is_control() { ' ' } else { c }));
        }
    }
    Some(result)
}
fn date(input: &mut &[u8]) -> Option<String> {
    let (tag, value) = item(input)?;
    let s = core::str::from_utf8(value).ok()?;
    let (year, tail) = match tag {
        23 if s.len() == 13 => {
            let y = s.get(..2)?.parse::<u32>().ok()?;
            (if y >= 50 { 1900 + y } else { 2000 + y }, s.get(2..)?)
        }
        24 if s.len() == 15 => (s.get(..4)?.parse().ok()?, s.get(4..)?),
        _ => return None,
    };
    if !tail.ends_with('Z') || !tail.as_bytes()[..10].iter().all(u8::is_ascii_digit) {
        return None;
    }
    Some(format!(
        "{year:04}-{}-{} {}:{}:{} UTC",
        &tail[..2],
        &tail[2..4],
        &tail[4..6],
        &tail[6..8],
        &tail[8..10]
    ))
}
/// Parse display fields from a DER certificate. `verified` must come from TLS.
pub fn parse_der(der: &[u8], verified: bool) -> Option<CertInfo> {
    if der.len() > 65536 {
        return None;
    }
    let mut outer = der;
    let mut cert = take(&mut outer, 0x30)?;
    if !outer.is_empty() {
        return None;
    }
    let mut tbs = take(&mut cert, 0x30)?;
    if tbs.first() == Some(&0xa0) {
        item(&mut tbs)?;
    }
    take(&mut tbs, 2)?;
    take(&mut tbs, 0x30)?;
    let issuer = name(take(&mut tbs, 0x30)?)?;
    let mut validity = take(&mut tbs, 0x30)?;
    let valid_from = date(&mut validity);
    let valid_to = date(&mut validity);
    let subject = name(take(&mut tbs, 0x30)?)?;
    let mut spki = take(&mut tbs, 0x30)?;
    let mut key_algorithm = take(&mut spki, 0x30)?;
    let key_oid = take(&mut key_algorithm, 6)?;
    let bits = take(&mut spki, 3)?;
    let key_bits = if key_oid == [42, 134, 72, 134, 247, 13, 1, 1, 1] {
        let mut key = bits.get(1..)?;
        let mut rsa = take(&mut key, 0x30)?;
        let modulus = take(&mut rsa, 2)?;
        let modulus = modulus.strip_prefix(&[0]).unwrap_or(modulus);
        modulus
            .first()
            .map(|b| (modulus.len() as u32) * 8 - b.leading_zeros())
    } else if key_oid == [43, 101, 112] {
        Some(256)
    } else if key_oid == [42, 134, 72, 206, 61, 2, 1] {
        match take(&mut key_algorithm, 6)? {
            [42, 134, 72, 206, 61, 3, 1, 7] => Some(256),
            [43, 129, 4, 0, 34] => Some(384),
            [43, 129, 4, 0, 35] => Some(521),
            _ => None,
        }
    } else {
        None
    };
    let mut algorithm = take(&mut cert, 0x30)?;
    let oid = take(&mut algorithm, 6)?;
    let signature_algorithm = Some(match oid {
        [42, 134, 72, 134, 247, 13, 1, 1, 11] => String::from("RSA / SHA-256"),
        [42, 134, 72, 134, 247, 13, 1, 1, 12] => String::from("RSA / SHA-384"),
        [42, 134, 72, 134, 247, 13, 1, 1, 13] => String::from("RSA / SHA-512"),
        [42, 134, 72, 206, 61, 4, 3, 2] => String::from("ECDSA / SHA-256"),
        [42, 134, 72, 206, 61, 4, 3, 3] => String::from("ECDSA / SHA-384"),
        [43, 101, 112] => String::from("Ed25519"),
        _ => format!("OID bytes: {oid:02x?}"),
    });
    take(&mut cert, 3)?;
    Some(CertInfo {
        verified_host: None,
        subject,
        issuer,
        valid_from,
        valid_to,
        signature_algorithm,
        key_bits,
        chain: Vec::new(),
        is_valid: verified,
        trust: CertificateTrust::Unverified,
        validation_errors: Vec::new(),
    })
}
/// Peer certificates are supplied leaf-first by rustls. Parsing never changes trust.
pub fn peer_chain<'a>(certs: impl IntoIterator<Item = &'a [u8]>) -> Option<CertInfo> {
    let mut certs = certs.into_iter();
    let mut leaf = parse_der(certs.next()?, true)?;
    leaf.trust = CertificateTrust::SessionSupplied;
    let mut chain = alloc::vec![leaf.clone()];
    for der in certs.take(15) {
        if let Some(mut cert) = parse_der(der, false) {
            cert.trust = CertificateTrust::SessionSupplied;
            chain.push(cert);
        }
    }
    chain.reverse();
    leaf.chain = chain;
    Some(leaf)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn kv_der_and_peer_session() {
        let der = include_bytes!("../../sunlight-tls/src/roots/gts_root_r4.der");
        let ca = parse_der(der, false).unwrap();
        assert!(ca.subject.contains("GTS Root R4"));
        assert!(!ca.is_valid);
        assert_eq!(ca.key_bits, Some(384));
        assert!(ca.valid_from.unwrap().ends_with("UTC"));
        let session = peer_chain([der.as_slice()]).unwrap();
        assert!(session.is_valid);
        assert_eq!(session.chain.len(), 1);
    }
    #[test]
    fn rejects_truncated_der() {
        let der = include_bytes!("../../sunlight-tls/src/roots/gts_root_r4.der");
        for end in 0..der.len() {
            assert!(parse_der(&der[..end], false).is_none());
        }
        assert!(parse_der(&[0x30, 0x80], false).is_none());
    }
}
