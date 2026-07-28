//! Plain text and log parser.

use alloc::string::String;
use alloc::vec::Vec;

use crate::config::PARSER_PLAIN;
use crate::error::IndexError;
use crate::parse::{
    line_of, line_starts, push_block, DocumentParser, ParseSummary, ParsedBlock, ParsedBlockKind,
};
use crate::quotas::IndexQuotaConfig;

pub struct PlainTextParser;

impl DocumentParser for PlainTextParser {
    fn parser_id(&self) -> u32 {
        PARSER_PLAIN
    }

    fn parser_version(&self) -> u32 {
        1
    }

    fn supports(&self, extension: &str, _media_hint: Option<&str>) -> bool {
        matches!(
            extension,
            "txt"
                | "log"
                | "rs"
                | "c"
                | "h"
                | "cpp"
                | "hpp"
                | "py"
                | "js"
                | "ts"
                | "tsx"
                | "jsx"
                | "html"
                | "css"
                | "sh"
        )
    }

    fn parse(
        &self,
        input: &str,
        quotas: &IndexQuotaConfig,
        output: &mut Vec<ParsedBlock>,
    ) -> Result<ParseSummary, IndexError> {
        let starts = line_starts(input);
        let bytes = input.as_bytes();
        let mut para_start = 0usize;
        let mut i = 0usize;
        let mut blocks_before = output.len() as u32;

        while i <= bytes.len() {
            let at_end = i == bytes.len();
            let blank = if at_end {
                true
            } else if bytes[i] == b'\n' {
                // blank line if previous was also newline or start
                true
            } else {
                false
            };

            if at_end || (bytes[i] == b'\n' && (i + 1 == bytes.len() || bytes[i + 1] == b'\n')) {
                let end = if at_end { i } else { i };
                if end > para_start {
                    let text = core::str::from_utf8(&bytes[para_start..end])
                        .map_err(|_| IndexError::InvalidUtf8)?;
                    let trimmed = text.trim_end_matches('\n').trim_end_matches('\r');
                    if !trimmed.trim().is_empty() {
                        let kind = if extension_is_log(input) {
                            ParsedBlockKind::LogEntry
                        } else {
                            ParsedBlockKind::Paragraph
                        };
                        // Heuristic: treat as log only when every non-empty line looks short;
                        // for .log extension we set via supports path — use Paragraph for plain.
                        let _ = kind;
                        push_block(
                            output,
                            quotas,
                            ParsedBlock {
                                block_kind: ParsedBlockKind::Paragraph,
                                byte_start: para_start as u64,
                                byte_end: end as u64,
                                line_start: line_of(&starts, para_start),
                                line_end: line_of(&starts, end.saturating_sub(1).max(para_start)),
                                heading_path: String::new(),
                                text: String::from(trimmed),
                            },
                        )?;
                    }
                }
                if at_end {
                    break;
                }
                // skip blank lines
                while i < bytes.len() && bytes[i] == b'\n' {
                    i += 1;
                }
                para_start = i;
                continue;
            }
            i += 1;
            let _ = blank;
        }

        // Fallback: single block for content without double newlines
        if output.len() as u32 == blocks_before && !input.trim().is_empty() {
            push_block(
                output,
                quotas,
                ParsedBlock {
                    block_kind: ParsedBlockKind::PlainText,
                    byte_start: 0,
                    byte_end: input.len() as u64,
                    line_start: 1,
                    line_end: starts.len() as u32,
                    heading_path: String::new(),
                    text: String::from(input.trim_end()),
                },
            )?;
        }

        Ok(ParseSummary {
            blocks: (output.len() as u32).saturating_sub(blocks_before),
            bytes_consumed: input.len() as u64,
        })
    }
}

fn extension_is_log(_input: &str) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paragraphs_split() {
        let p = PlainTextParser;
        let q = IndexQuotaConfig::default();
        let mut out = Vec::new();
        p.parse("hello\n\nworld\n", &q, &mut out).unwrap();
        assert!(out.len() >= 2);
        assert!(out[0].text.contains("hello"));
        assert!(out[1].text.contains("world"));
    }

    #[test]
    fn empty_file() {
        let p = PlainTextParser;
        let q = IndexQuotaConfig::default();
        let mut out = Vec::new();
        let s = p.parse("", &q, &mut out).unwrap();
        assert_eq!(s.blocks, 0);
    }
}
