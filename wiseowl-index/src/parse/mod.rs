//! Document parsers producing structured text blocks (no summaries, no truth claims).

mod csv;
mod markdown;
mod plain;
mod structured;

use alloc::string::String;
use alloc::vec::Vec;

use crate::config::{
    PARSER_CSV, PARSER_MARKDOWN, PARSER_PLAIN, PARSER_STRUCTURED, PARSER_YAML_TEXT,
};
use crate::error::IndexError;
use crate::quotas::IndexQuotaConfig;

pub use csv::CsvParser;
pub use markdown::MarkdownParser;
pub use plain::PlainTextParser;
pub use structured::{JsonParser, TomlParser, YamlTextParser};

/// Kind of a parsed block (structural, not semantic truth).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "host", derive(serde::Serialize, serde::Deserialize))]
#[repr(u8)]
pub enum ParsedBlockKind {
    PlainText = 1,
    Paragraph = 2,
    Heading = 3,
    ListItem = 4,
    CodeBlock = 5,
    TableRow = 6,
    KeyValue = 7,
    LogEntry = 8,
}

/// One structured text block with source ranges.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedBlock {
    pub block_kind: ParsedBlockKind,
    pub byte_start: u64,
    pub byte_end: u64,
    pub line_start: u32,
    pub line_end: u32,
    pub heading_path: String,
    pub text: String,
}

/// Parse summary (counts only).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ParseSummary {
    pub blocks: u32,
    pub bytes_consumed: u64,
}

/// Parser interface.
pub trait DocumentParser {
    fn parser_id(&self) -> u32;
    fn parser_version(&self) -> u32;
    fn supports(&self, extension: &str, media_hint: Option<&str>) -> bool;
    fn parse(
        &self,
        input: &str,
        quotas: &IndexQuotaConfig,
        output: &mut Vec<ParsedBlock>,
    ) -> Result<ParseSummary, IndexError>;
}

/// Select a parser for an extension.
pub fn select_parser(extension: &str) -> Option<&'static dyn DocumentParser> {
    static PLAIN: PlainTextParser = PlainTextParser;
    static MD: MarkdownParser = MarkdownParser;
    static JSON: JsonParser = JsonParser;
    static TOML: TomlParser = TomlParser;
    static YAML: YamlTextParser = YamlTextParser;
    static CSV: CsvParser = CsvParser;

    let ext = extension;
    if PLAIN.supports(ext, None) {
        return Some(&PLAIN);
    }
    if MD.supports(ext, None) {
        return Some(&MD);
    }
    if JSON.supports(ext, None) {
        return Some(&JSON);
    }
    if TOML.supports(ext, None) {
        return Some(&TOML);
    }
    if YAML.supports(ext, None) {
        return Some(&YAML);
    }
    if CSV.supports(ext, None) {
        return Some(&CSV);
    }
    None
}

/// Helper: push a block if non-empty after trim, enforcing quotas.
pub(crate) fn push_block(
    out: &mut Vec<ParsedBlock>,
    quotas: &IndexQuotaConfig,
    block: ParsedBlock,
) -> Result<(), IndexError> {
    if block.text.trim().is_empty() {
        return Ok(());
    }
    if out.len() as u32 >= quotas.max_blocks_per_file {
        return Err(IndexError::QuotaExceeded("blocks per file"));
    }
    if block.text.len() as u32 > quotas.max_bytes_per_chunk.saturating_mul(4) {
        // Split oversized text into multiple plain blocks.
        let max = quotas.max_bytes_per_chunk as usize;
        let bytes = block.text.as_bytes();
        let mut start = 0usize;
        let mut ord = 0u32;
        while start < bytes.len() {
            let end = (start + max).min(bytes.len());
            // Prefer split on newline.
            let mut split = end;
            if end < bytes.len() {
                if let Some(rel) = bytes[start..end].iter().rposition(|&b| b == b'\n') {
                    split = start + rel + 1;
                }
            }
            if split == start {
                split = end;
            }
            let slice =
                core::str::from_utf8(&bytes[start..split]).map_err(|_| IndexError::InvalidUtf8)?;
            if !slice.trim().is_empty() {
                if out.len() as u32 >= quotas.max_blocks_per_file {
                    return Err(IndexError::QuotaExceeded("blocks per file"));
                }
                out.push(ParsedBlock {
                    block_kind: block.block_kind,
                    byte_start: block.byte_start + start as u64,
                    byte_end: block.byte_start + split as u64,
                    line_start: block.line_start.saturating_add(ord),
                    line_end: block.line_start.saturating_add(ord),
                    heading_path: block.heading_path.clone(),
                    text: String::from(slice),
                });
                ord = ord.saturating_add(1);
            }
            start = split;
        }
        return Ok(());
    }
    out.push(block);
    Ok(())
}

/// Line-oriented byte/line helpers for original text (with normalized LF).
pub(crate) fn line_starts(text: &str) -> Vec<usize> {
    let mut starts = Vec::new();
    starts.push(0);
    for (i, b) in text.bytes().enumerate() {
        if b == b'\n' && i + 1 < text.len() {
            starts.push(i + 1);
        }
    }
    starts
}

pub(crate) fn line_of(starts: &[usize], byte: usize) -> u32 {
    match starts.binary_search(&byte) {
        Ok(i) => i as u32 + 1,
        Err(i) => i as u32, // 1-based: i is insertion point
    }
    .max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn select_md() {
        assert_eq!(select_parser("md").unwrap().parser_id(), PARSER_MARKDOWN);
    }

    #[test]
    fn select_json() {
        assert_eq!(
            select_parser("json").unwrap().parser_id(),
            PARSER_STRUCTURED
        );
    }

    #[test]
    fn select_csv() {
        assert_eq!(select_parser("csv").unwrap().parser_id(), PARSER_CSV);
    }

    #[test]
    fn select_yaml() {
        assert_eq!(select_parser("yml").unwrap().parser_id(), PARSER_YAML_TEXT);
    }
}
