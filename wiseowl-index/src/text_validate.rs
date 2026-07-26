//! UTF-8 text validation and binary heuristics.
//!
//! Strict UTF-8 for Phase 3. Invalid UTF-8 is rejected (no silent corruption import).
//! Original payload is never mutated for storage; normalization is tokenizer-only.

use crate::error::IndexError;
use crate::quotas::IndexQuotaConfig;

/// Strip UTF-8 BOM if present; return remaining bytes.
pub fn strip_utf8_bom(data: &[u8]) -> &[u8] {
    if data.starts_with(&[0xEF, 0xBB, 0xBF]) {
        &data[3..]
    } else {
        data
    }
}

/// Validate that bytes are acceptable UTF-8 text for Phase 3 ingestion.
pub fn validate_utf8_text<'a>(
    data: &'a [u8],
    quotas: &IndexQuotaConfig,
) -> Result<&'a str, IndexError> {
    let data = strip_utf8_bom(data);
    if data.is_empty() {
        return Ok("");
    }
    // Embedded NUL → binary.
    if data.contains(&0) {
        return Err(IndexError::BinaryContent);
    }
    // Binary heuristic: high ratio of non-text control bytes in first window.
    if looks_binary(data) {
        return Err(IndexError::BinaryContent);
    }
    let text = core::str::from_utf8(data).map_err(|_| IndexError::InvalidUtf8)?;
    // Line length bound (soft check on max line).
    let mut line_start = 0usize;
    for (i, b) in text.bytes().enumerate() {
        if b == b'\n' {
            let len = i.saturating_sub(line_start) as u32;
            // Allow CRLF: the \r is inside the line measure; still fine if under limit.
            if len > quotas.max_line_bytes {
                return Err(IndexError::QuotaExceeded("line length"));
            }
            line_start = i + 1;
        }
    }
    let tail = text.len().saturating_sub(line_start) as u32;
    if tail > quotas.max_line_bytes {
        return Err(IndexError::QuotaExceeded("line length"));
    }
    // Excessive control characters (excluding \t \n \r).
    let mut controls = 0u32;
    for b in text.bytes() {
        if b < 0x20 && b != b'\t' && b != b'\n' && b != b'\r' {
            controls = controls.saturating_add(1);
        }
    }
    if !text.is_empty() && controls as usize * 10 > text.len() {
        return Err(IndexError::BinaryContent);
    }
    Ok(text)
}

fn looks_binary(data: &[u8]) -> bool {
    let window = data.len().min(512);
    if window == 0 {
        return false;
    }
    let mut suspicious = 0u32;
    for &b in &data[..window] {
        if b == 0 {
            return true;
        }
        // DEL and C0 controls except tab/lf/cr
        if b < 0x09 || (b > 0x0d && b < 0x20) || b == 0x7f {
            suspicious = suspicious.saturating_add(1);
        }
    }
    // > 10% suspicious in sample → binary-like
    suspicious * 10 > window as u32
}

/// Normalize newlines for parser convenience: CRLF → LF. Does not affect stored payload.
pub fn normalize_newlines_owned(text: &str) -> alloc::string::String {
    let mut out = alloc::string::String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\r' {
            if i + 1 < bytes.len() && bytes[i + 1] == b'\n' {
                out.push('\n');
                i += 2;
                continue;
            }
            out.push('\n');
            i += 1;
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_ok() {
        let q = IndexQuotaConfig::default();
        assert_eq!(validate_utf8_text(b"", &q).unwrap(), "");
    }

    #[test]
    fn bom_stripped() {
        let q = IndexQuotaConfig::default();
        let mut v = vec![0xEF, 0xBB, 0xBF];
        v.extend_from_slice(b"hi");
        assert_eq!(validate_utf8_text(&v, &q).unwrap(), "hi");
    }

    #[test]
    fn crlf_ok() {
        let q = IndexQuotaConfig::default();
        assert!(validate_utf8_text(b"a\r\nb\n", &q).is_ok());
    }

    #[test]
    fn nul_rejected() {
        let q = IndexQuotaConfig::default();
        assert!(matches!(
            validate_utf8_text(b"a\0b", &q),
            Err(IndexError::BinaryContent)
        ));
    }

    #[test]
    fn invalid_utf8_rejected() {
        let q = IndexQuotaConfig::default();
        assert!(matches!(
            validate_utf8_text(&[0xFF, 0xFE], &q),
            Err(IndexError::InvalidUtf8) | Err(IndexError::BinaryContent)
        ));
    }

    #[test]
    fn binary_heuristic() {
        let q = IndexQuotaConfig::default();
        let mut bin = vec![0u8; 64];
        for i in 0..64 {
            bin[i] = i as u8;
        }
        assert!(matches!(
            validate_utf8_text(&bin, &q),
            Err(IndexError::BinaryContent)
        ));
    }
}
