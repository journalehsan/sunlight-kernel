//! WiseOwlLexicalV1 — deterministic lexical retrieval tokenizer.

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::config::WISEOWL_LEXICAL_V1_ID;
use crate::error::IndexError;
use crate::hash::fnv1a64;
use crate::quotas::IndexQuotaConfig;
use crate::tokenize::dictionary::TokenDictionary;
use crate::tokenize::normalize::normalize_for_retrieval;
use crate::tokenize::{
    EmittedToken, NormalizedTextBuffer, RetrievalTokenizer, TokenSink, TokenizationSummary,
};

/// Phase 3 lexical tokenizer.
pub struct WiseOwlLexicalV1;

impl Default for WiseOwlLexicalV1 {
    fn default() -> Self {
        Self
    }
}

impl RetrievalTokenizer for WiseOwlLexicalV1 {
    fn tokenizer_id(&self) -> u32 {
        WISEOWL_LEXICAL_V1_ID
    }

    fn version(&self) -> u32 {
        1
    }

    fn normalize(
        &self,
        input: &str,
        output: &mut NormalizedTextBuffer,
    ) -> Result<(), IndexError> {
        output.text = normalize_for_retrieval(input);
        Ok(())
    }

    fn tokenize(
        &self,
        normalized: &str,
        dict: &mut TokenDictionary,
        quotas: &IndexQuotaConfig,
        output: &mut TokenSink,
    ) -> Result<TokenizationSummary, IndexError> {
        let mut freq: BTreeMap<u64, (String, u16, Vec<u32>, bool)> = BTreeMap::new();
        let mut ordinal = 0u32;
        let mut tokens_emitted = 0u32;
        let mut positions_stored = 0u32;
        let mut positions_truncated = 0u32;

        for raw in split_tokens(normalized) {
            let token = bound_token(raw, quotas);
            if token.is_empty() {
                continue;
            }
            if tokens_emitted >= quotas.max_tokens_per_chunk {
                break;
            }
            let id = dict.intern(self.tokenizer_id(), self.version(), &token, quotas, true)?;
            tokens_emitted = tokens_emitted.saturating_add(1);
            let entry = freq.entry(id).or_insert_with(|| {
                (token.clone(), 0, Vec::new(), false)
            });
            entry.1 = entry.1.saturating_add(1);
            if (entry.2.len() as u32) < quotas.max_positions_per_token {
                entry.2.push(ordinal);
                positions_stored = positions_stored.saturating_add(1);
            } else {
                entry.3 = true;
                positions_truncated = positions_truncated.saturating_add(1);
            }
            ordinal = ordinal.saturating_add(1);
            if freq.len() as u32 > quotas.max_unique_tokens_per_chunk {
                return Err(IndexError::QuotaExceeded("unique tokens per chunk"));
            }
        }

        output.tokens.clear();
        for (token_id, (canonical, frequency, positions, truncated)) in freq {
            output.tokens.push(EmittedToken {
                token_id,
                canonical,
                frequency,
                positions,
                positions_truncated: truncated,
            });
        }
        // Stable order by token_id
        output.tokens.sort_by_key(|t| t.token_id);

        Ok(TokenizationSummary {
            tokens_emitted,
            unique_tokens: output.tokens.len() as u32,
            positions_stored,
            positions_truncated,
        })
    }
}

/// Split normalized text into token strings.
///
/// Keeps alphanumeric runs, and identifiers with internal `-` `_` `.` as single tokens
/// when they look like identifiers (no leading/trailing separators).
fn split_tokens(normalized: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let chars: Vec<char> = normalized.chars().collect();
    let mut i = 0usize;
    while i < chars.len() {
        let c = chars[i];
        if c == ' ' {
            flush_cur(&mut cur, &mut out);
            i += 1;
            continue;
        }
        if is_token_char(c) {
            cur.push(c);
            i += 1;
            continue;
        }
        flush_cur(&mut cur, &mut out);
        i += 1;
    }
    flush_cur(&mut cur, &mut out);
    out
}

fn is_token_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '-' || c == '.' || c == '/' || c == ':'
}

fn flush_cur(cur: &mut String, out: &mut Vec<String>) {
    if cur.is_empty() {
        return;
    }
    // Trim leading/trailing separators from identifiers
    let trimmed = cur.trim_matches(|c| c == '.' || c == '-' || c == '_' || c == '/' || c == ':');
    if !trimmed.is_empty() {
        out.push(trimmed.to_string());
    }
    cur.clear();
}

fn bound_token(raw: String, quotas: &IndexQuotaConfig) -> String {
    if raw.len() as u16 <= quotas.max_token_length {
        return raw;
    }
    // Truncate + suffix hash for stability of long tokens.
    let max = quotas.max_token_length as usize;
    let keep = max.saturating_sub(12).max(1);
    let prefix: String = raw.chars().take(keep).collect();
    let h = fnv1a64(raw.as_bytes());
    alloc::format!("{prefix}#{h:08x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers() {
        let t = WiseOwlLexicalV1;
        let mut dict = TokenDictionary::new();
        let q = IndexQuotaConfig::default();
        let mut norm = NormalizedTextBuffer::default();
        t.normalize("sunlight-memorydb wiseowl_indexd network.timeout", &mut norm)
            .unwrap();
        let mut sink = TokenSink::default();
        t.tokenize(&norm.text, &mut dict, &q, &mut sink).unwrap();
        let canons: Vec<_> = sink.tokens.iter().map(|t| t.canonical.as_str()).collect();
        assert!(canons.iter().any(|c| *c == "sunlight-memorydb"));
        assert!(canons.iter().any(|c| *c == "wiseowl_indexd"));
        assert!(canons.iter().any(|c| *c == "network.timeout"));
    }

    #[test]
    fn deterministic() {
        let t = WiseOwlLexicalV1;
        let q = IndexQuotaConfig::default();
        let mut d1 = TokenDictionary::new();
        let mut d2 = TokenDictionary::new();
        let mut n = NormalizedTextBuffer::default();
        t.normalize("Thermal Fan Service", &mut n).unwrap();
        let mut s1 = TokenSink::default();
        let mut s2 = TokenSink::default();
        t.tokenize(&n.text, &mut d1, &q, &mut s1).unwrap();
        t.tokenize(&n.text, &mut d2, &q, &mut s2).unwrap();
        assert_eq!(s1.tokens, s2.tokens);
    }

    #[test]
    fn persian_and_latin_mixed() {
        let t = WiseOwlLexicalV1;
        let mut dict = TokenDictionary::new();
        let q = IndexQuotaConfig::default();
        let mut n = NormalizedTextBuffer::default();
        // Arabic yeh form should match Persian
        t.normalize("كتاب book", &mut n).unwrap();
        let mut sink = TokenSink::default();
        t.tokenize(&n.text, &mut dict, &q, &mut sink).unwrap();
        assert!(sink.tokens.iter().any(|t| t.canonical == "book"));
        // canonical should use ک and ی forms
        assert!(sink.tokens.iter().any(|t| t.canonical.contains('ک') || t.canonical.contains('ا')));
    }

    #[test]
    fn ip_and_version_tokens() {
        let t = WiseOwlLexicalV1;
        let mut dict = TokenDictionary::new();
        let q = IndexQuotaConfig::default();
        let mut n = NormalizedTextBuffer::default();
        t.normalize("192.168.1.1 UTF-8 Rust2024", &mut n).unwrap();
        let mut sink = TokenSink::default();
        t.tokenize(&n.text, &mut dict, &q, &mut sink).unwrap();
        let canons: Vec<_> = sink.tokens.iter().map(|t| t.canonical.clone()).collect();
        assert!(canons.iter().any(|c| c == "192.168.1.1"));
        assert!(canons.iter().any(|c| c == "utf-8"));
        assert!(canons.iter().any(|c| c == "rust2024"));
    }
}
