//! Lexical retrieval tokenizer (not a model tokenizer).

mod dictionary;
mod lexical;
mod normalize;

use alloc::string::String;
use alloc::vec::Vec;

use crate::error::IndexError;
use crate::quotas::IndexQuotaConfig;

pub use dictionary::{TokenDictionary, TokenDictionaryEntry};
pub use lexical::WiseOwlLexicalV1;
pub use normalize::normalize_for_retrieval;

/// Emitted token with optional position (normalized ordinal).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmittedToken {
    pub token_id: u64,
    pub canonical: String,
    pub frequency: u16,
    pub positions: Vec<u32>,
    pub positions_truncated: bool,
}

/// Tokenization summary counters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TokenizationSummary {
    pub tokens_emitted: u32,
    pub unique_tokens: u32,
    pub positions_stored: u32,
    pub positions_truncated: u32,
}

/// Sink for tokenizer output.
pub struct TokenSink {
    pub tokens: Vec<EmittedToken>,
}

impl Default for TokenSink {
    fn default() -> Self {
        Self { tokens: Vec::new() }
    }
}

/// Normalized text buffer (separate from original payload).
pub struct NormalizedTextBuffer {
    pub text: String,
}

impl Default for NormalizedTextBuffer {
    fn default() -> Self {
        Self {
            text: String::new(),
        }
    }
}

/// Retrieval tokenizer trait.
pub trait RetrievalTokenizer {
    fn tokenizer_id(&self) -> u32;
    fn version(&self) -> u32;
    fn normalize(
        &self,
        input: &str,
        output: &mut NormalizedTextBuffer,
    ) -> Result<(), IndexError>;
    fn tokenize(
        &self,
        normalized: &str,
        dict: &mut TokenDictionary,
        quotas: &IndexQuotaConfig,
        output: &mut TokenSink,
    ) -> Result<TokenizationSummary, IndexError>;
}

/// Convert sink to memorydb IndexedToken list (sorted later by normalize_tokens).
pub fn to_indexed_tokens(
    sink: &TokenSink,
) -> Vec<wiseowl_memorydb::tokens::IndexedToken> {
    sink.tokens
        .iter()
        .map(|t| {
            let positions = if t.positions.is_empty() {
                None
            } else {
                Some(wiseowl_memorydb::tokens::BoundedPositions {
                    positions: t.positions.clone(),
                })
            };
            wiseowl_memorydb::tokens::IndexedToken {
                token_id: t.token_id,
                frequency: t.frequency.max(1),
                positions,
            }
        })
        .collect()
}
