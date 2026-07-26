//! Markdown and reStructuredText-ish structural parser (deterministic, no execution).

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::config::PARSER_MARKDOWN;
use crate::error::IndexError;
use crate::parse::{line_of, line_starts, push_block, DocumentParser, ParseSummary, ParsedBlock, ParsedBlockKind};
use crate::quotas::IndexQuotaConfig;

pub struct MarkdownParser;

impl DocumentParser for MarkdownParser {
    fn parser_id(&self) -> u32 {
        PARSER_MARKDOWN
    }

    fn parser_version(&self) -> u32 {
        1
    }

    fn supports(&self, extension: &str, _media_hint: Option<&str>) -> bool {
        matches!(extension, "md" | "rst")
    }

    fn parse(
        &self,
        input: &str,
        quotas: &IndexQuotaConfig,
        output: &mut Vec<ParsedBlock>,
    ) -> Result<ParseSummary, IndexError> {
        let starts = line_starts(input);
        let lines: Vec<&str> = input.split('\n').collect();
        let mut heading_stack: Vec<String> = Vec::new();
        let mut para = String::new();
        let mut para_byte_start = 0usize;
        let mut para_line_start = 1u32;
        let mut in_code = false;
        let mut code = String::new();
        let mut code_byte_start = 0usize;
        let mut code_line_start = 1u32;
        let mut byte_pos = 0usize;
        let mut blocks_before = output.len() as u32;

        for (li, line) in lines.iter().enumerate() {
            let line_no = (li as u32) + 1;
            let line_byte = byte_pos;
            let is_fence = line.trim_start().starts_with("```")
                || (line.trim_start().starts_with("~~~"));

            if is_fence {
                if in_code {
                    push_block(
                        output,
                        quotas,
                        ParsedBlock {
                            block_kind: ParsedBlockKind::CodeBlock,
                            byte_start: code_byte_start as u64,
                            byte_end: line_byte as u64,
                            line_start: code_line_start,
                            line_end: line_no,
                            heading_path: heading_path(&heading_stack, quotas),
                            text: code.clone(),
                        },
                    )?;
                    code.clear();
                    in_code = false;
                } else {
                    flush_para(
                        output,
                        quotas,
                        &mut para,
                        para_byte_start,
                        para_line_start,
                        line_no,
                        &heading_stack,
                    )?;
                    in_code = true;
                    code_byte_start = line_byte + line.len() + 1; // after fence line
                    code_line_start = line_no + 1;
                }
                byte_pos += line.len() + 1;
                continue;
            }

            if in_code {
                if !code.is_empty() {
                    code.push('\n');
                }
                code.push_str(line);
                byte_pos += line.len() + 1;
                continue;
            }

            // ATX heading
            if let Some(h) = parse_atx_heading(line) {
                flush_para(
                    output,
                    quotas,
                    &mut para,
                    para_byte_start,
                    para_line_start,
                    line_no,
                    &heading_stack,
                )?;
                let level = h.0.min(quotas.max_heading_depth as usize);
                while heading_stack.len() >= level {
                    heading_stack.pop();
                }
                heading_stack.push(h.1);
                push_block(
                    output,
                    quotas,
                    ParsedBlock {
                        block_kind: ParsedBlockKind::Heading,
                        byte_start: line_byte as u64,
                        byte_end: (line_byte + line.len()) as u64,
                        line_start: line_no,
                        line_end: line_no,
                        heading_path: heading_path(&heading_stack, quotas),
                        text: heading_stack.last().cloned().unwrap_or_default(),
                    },
                )?;
                byte_pos += line.len() + 1;
                continue;
            }

            // reST underline heading: previous non-empty line + === or ---
            if li > 0
                && is_rst_underline(line)
                && !lines[li - 1].trim().is_empty()
                && !lines[li - 1].starts_with('#')
            {
                // Already may have been flushed as para; emit heading from previous line.
                let title = lines[li - 1].trim().to_string();
                if !title.is_empty() {
                    heading_stack.clear();
                    heading_stack.push(title.clone());
                    // Fix last paragraph if it matches — best-effort: emit heading.
                    push_block(
                        output,
                        quotas,
                        ParsedBlock {
                            block_kind: ParsedBlockKind::Heading,
                            byte_start: line_of_byte(&starts, li - 1) as u64,
                            byte_end: (line_byte + line.len()) as u64,
                            line_start: line_no.saturating_sub(1).max(1),
                            line_end: line_no,
                            heading_path: heading_path(&heading_stack, quotas),
                            text: title,
                        },
                    )?;
                }
                byte_pos += line.len() + 1;
                continue;
            }

            // List items
            if is_list_item(line) {
                flush_para(
                    output,
                    quotas,
                    &mut para,
                    para_byte_start,
                    para_line_start,
                    line_no,
                    &heading_stack,
                )?;
                let text = strip_list_marker(line);
                push_block(
                    output,
                    quotas,
                    ParsedBlock {
                        block_kind: ParsedBlockKind::ListItem,
                        byte_start: line_byte as u64,
                        byte_end: (line_byte + line.len()) as u64,
                        line_start: line_no,
                        line_end: line_no,
                        heading_path: heading_path(&heading_stack, quotas),
                        text,
                    },
                )?;
                byte_pos += line.len() + 1;
                continue;
            }

            if line.trim().is_empty() {
                flush_para(
                    output,
                    quotas,
                    &mut para,
                    para_byte_start,
                    para_line_start,
                    line_no,
                    &heading_stack,
                )?;
            } else {
                if para.is_empty() {
                    para_byte_start = line_byte;
                    para_line_start = line_no;
                } else {
                    para.push('\n');
                }
                // Strip light markdown emphasis markers for retrieval noise reduction only
                // inside block text for indexing — keep mostly raw.
                para.push_str(line);
            }
            byte_pos += line.len() + 1;
        }

        if in_code && !code.trim().is_empty() {
            push_block(
                output,
                quotas,
                ParsedBlock {
                    block_kind: ParsedBlockKind::CodeBlock,
                    byte_start: code_byte_start as u64,
                    byte_end: input.len() as u64,
                    line_start: code_line_start,
                    line_end: lines.len() as u32,
                    heading_path: heading_path(&heading_stack, quotas),
                    text: code,
                },
            )?;
        }
        flush_para(
            output,
            quotas,
            &mut para,
            para_byte_start,
            para_line_start,
            lines.len() as u32,
            &heading_stack,
        )?;

        Ok(ParseSummary {
            blocks: (output.len() as u32).saturating_sub(blocks_before),
            bytes_consumed: input.len() as u64,
        })
    }
}

fn flush_para(
    out: &mut Vec<ParsedBlock>,
    quotas: &IndexQuotaConfig,
    para: &mut String,
    byte_start: usize,
    line_start: u32,
    line_end: u32,
    headings: &[String],
) -> Result<(), IndexError> {
    if para.trim().is_empty() {
        para.clear();
        return Ok(());
    }
    let text = para.trim().to_string();
    let end = byte_start + para.len();
    push_block(
        out,
        quotas,
        ParsedBlock {
            block_kind: ParsedBlockKind::Paragraph,
            byte_start: byte_start as u64,
            byte_end: end as u64,
            line_start,
            line_end: line_end.max(line_start),
            heading_path: heading_path(headings, quotas),
            text,
        },
    )?;
    para.clear();
    Ok(())
}

fn parse_atx_heading(line: &str) -> Option<(usize, String)> {
    let t = line.trim_start();
    if !t.starts_with('#') {
        return None;
    }
    let mut level = 0usize;
    for c in t.chars() {
        if c == '#' {
            level += 1;
        } else {
            break;
        }
    }
    if level == 0 || level > 6 {
        return None;
    }
    let rest = t[level..].trim();
    if rest.is_empty() {
        return None;
    }
    Some((level, rest.to_string()))
}

fn is_rst_underline(line: &str) -> bool {
    let t = line.trim();
    if t.len() < 3 {
        return false;
    }
    let b = t.as_bytes()[0];
    if !matches!(b, b'=' | b'-' | b'~' | b'^') {
        return false;
    }
    t.bytes().all(|c| c == b)
}

fn is_list_item(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with("- ")
        || t.starts_with("* ")
        || t.starts_with("+ ")
        || (t.len() >= 3
            && t.as_bytes()[0].is_ascii_digit()
            && t.contains(". "))
}

fn strip_list_marker(line: &str) -> String {
    let t = line.trim_start();
    if let Some(r) = t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")).or_else(|| t.strip_prefix("+ ")) {
        return r.to_string();
    }
    if let Some(idx) = t.find(". ") {
        if t[..idx].bytes().all(|b| b.is_ascii_digit()) {
            return t[idx + 2..].to_string();
        }
    }
    t.to_string()
}

fn heading_path(stack: &[String], quotas: &IndexQuotaConfig) -> String {
    let mut s = stack.join("/");
    if s.len() as u16 > quotas.max_heading_path_bytes {
        s.truncate(quotas.max_heading_path_bytes as usize);
    }
    s
}

fn line_of_byte(starts: &[usize], line_idx: usize) -> usize {
    starts.get(line_idx).copied().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headings_and_code() {
        let p = MarkdownParser;
        let q = IndexQuotaConfig::default();
        let mut out = Vec::new();
        let src = "# Title\n\nHello\n\n```\ncode\n```\n";
        p.parse(src, &q, &mut out).unwrap();
        assert!(out.iter().any(|b| b.block_kind == ParsedBlockKind::Heading));
        assert!(out.iter().any(|b| b.block_kind == ParsedBlockKind::CodeBlock));
        assert!(out.iter().any(|b| b.block_kind == ParsedBlockKind::Paragraph));
    }
}
