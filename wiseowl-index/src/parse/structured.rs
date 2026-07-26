//! JSON / TOML key-path blocks and YAML plain-text fallback.
//!
//! YAML: Phase 3 uses normalized plain text only (no alias expansion / unsafe loaders).
//! JSON / TOML: bounded nesting, no execution.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::config::{PARSER_STRUCTURED, PARSER_YAML_TEXT};
use crate::error::IndexError;
use crate::parse::{push_block, DocumentParser, ParseSummary, ParsedBlock, ParsedBlockKind};
use crate::quotas::IndexQuotaConfig;

pub struct JsonParser;
pub struct TomlParser;
pub struct YamlTextParser;

impl DocumentParser for JsonParser {
    fn parser_id(&self) -> u32 {
        PARSER_STRUCTURED
    }
    fn parser_version(&self) -> u32 {
        1
    }
    fn supports(&self, extension: &str, _: Option<&str>) -> bool {
        extension == "json"
    }
    fn parse(
        &self,
        input: &str,
        quotas: &IndexQuotaConfig,
        output: &mut Vec<ParsedBlock>,
    ) -> Result<ParseSummary, IndexError> {
        let mut entries = 0u32;
        let before = output.len() as u32;
        walk_json_like(input.trim(), "", 0, quotas, output, &mut entries)?;
        if output.len() as u32 == before && !input.trim().is_empty() {
            // Malformed: do not import partial corrupted structured data as structured;
            // reject.
            return Err(IndexError::ParseFailed("json"));
        }
        Ok(ParseSummary {
            blocks: (output.len() as u32).saturating_sub(before),
            bytes_consumed: input.len() as u64,
        })
    }
}

impl DocumentParser for TomlParser {
    fn parser_id(&self) -> u32 {
        PARSER_STRUCTURED
    }
    fn parser_version(&self) -> u32 {
        1
    }
    fn supports(&self, extension: &str, _: Option<&str>) -> bool {
        extension == "toml"
    }
    fn parse(
        &self,
        input: &str,
        quotas: &IndexQuotaConfig,
        output: &mut Vec<ParsedBlock>,
    ) -> Result<ParseSummary, IndexError> {
        let before = output.len() as u32;
        let mut section = String::new();
        let mut line_no = 0u32;
        for line in input.lines() {
            line_no = line_no.saturating_add(1);
            let t = line.trim();
            if t.is_empty() || t.starts_with('#') {
                continue;
            }
            if t.starts_with('[') && t.ends_with(']') {
                section = t.trim_matches(|c| c == '[' || c == ']').to_string();
                continue;
            }
            if let Some((k, v)) = t.split_once('=') {
                let key = k.trim();
                let val = v.trim().trim_matches('"').trim_matches('\'');
                if key.is_empty() {
                    continue;
                }
                let path = if section.is_empty() {
                    key.to_string()
                } else {
                    alloc::format!("{section}.{key}")
                };
                let text = alloc::format!("{path} = {val}");
                if text.len() as u16 > quotas.max_scalar_bytes.saturating_mul(2) {
                    continue;
                }
                push_block(
                    output,
                    quotas,
                    ParsedBlock {
                        block_kind: ParsedBlockKind::KeyValue,
                        byte_start: 0,
                        byte_end: 0,
                        line_start: line_no,
                        line_end: line_no,
                        heading_path: section.clone(),
                        text,
                    },
                )?;
            }
        }
        Ok(ParseSummary {
            blocks: (output.len() as u32).saturating_sub(before),
            bytes_consumed: input.len() as u64,
        })
    }
}

impl DocumentParser for YamlTextParser {
    fn parser_id(&self) -> u32 {
        PARSER_YAML_TEXT
    }
    fn parser_version(&self) -> u32 {
        1
    }
    fn supports(&self, extension: &str, _: Option<&str>) -> bool {
        matches!(extension, "yaml" | "yml")
    }
    fn parse(
        &self,
        input: &str,
        quotas: &IndexQuotaConfig,
        output: &mut Vec<ParsedBlock>,
    ) -> Result<ParseSummary, IndexError> {
        // Explicit limitation: no YAML alias expansion; plain text blocks only.
        // Reject obvious alias bombs (`&` / `*` dense patterns beyond bound).
        let mut anchors = 0u32;
        for b in input.bytes() {
            if b == b'&' || b == b'*' {
                anchors = anchors.saturating_add(1);
            }
        }
        if anchors > 32 {
            return Err(IndexError::ParseFailed("yaml alias density"));
        }
        let before = output.len() as u32;
        let mut para = String::new();
        let mut line_no = 0u32;
        let mut start_line = 1u32;
        for line in input.lines() {
            line_no = line_no.saturating_add(1);
            if line.trim().is_empty() {
                if !para.trim().is_empty() {
                    push_block(
                        output,
                        quotas,
                        ParsedBlock {
                            block_kind: ParsedBlockKind::PlainText,
                            byte_start: 0,
                            byte_end: 0,
                            line_start: start_line,
                            line_end: line_no,
                            heading_path: String::new(),
                            text: para.trim().to_string(),
                        },
                    )?;
                    para.clear();
                }
                start_line = line_no + 1;
            } else {
                if para.is_empty() {
                    start_line = line_no;
                } else {
                    para.push('\n');
                }
                para.push_str(line);
            }
        }
        if !para.trim().is_empty() {
            push_block(
                output,
                quotas,
                ParsedBlock {
                    block_kind: ParsedBlockKind::PlainText,
                    byte_start: 0,
                    byte_end: 0,
                    line_start: start_line,
                    line_end: line_no,
                    heading_path: String::new(),
                    text: para.trim().to_string(),
                },
            )?;
        }
        Ok(ParseSummary {
            blocks: (output.len() as u32).saturating_sub(before),
            bytes_consumed: input.len() as u64,
        })
    }
}

/// Minimal bounded JSON walker (objects/arrays/scalars) → key-path text blocks.
fn walk_json_like(
    s: &str,
    path: &str,
    depth: u16,
    quotas: &IndexQuotaConfig,
    out: &mut Vec<ParsedBlock>,
    entries: &mut u32,
) -> Result<(), IndexError> {
    if depth > quotas.max_parser_nesting {
        return Err(IndexError::QuotaExceeded("json nesting"));
    }
    let s = s.trim();
    if s.is_empty() {
        return Err(IndexError::ParseFailed("empty json"));
    }
    if s.starts_with('{') {
        return walk_object(s, path, depth, quotas, out, entries);
    }
    if s.starts_with('[') {
        return walk_array(s, path, depth, quotas, out, entries);
    }
    // scalar
    emit_kv(path, s, quotas, out, entries)
}

fn emit_kv(
    path: &str,
    value: &str,
    quotas: &IndexQuotaConfig,
    out: &mut Vec<ParsedBlock>,
    entries: &mut u32,
) -> Result<(), IndexError> {
    if *entries >= quotas.max_json_entries {
        return Err(IndexError::QuotaExceeded("json entries"));
    }
    *entries = entries.saturating_add(1);
    let val = value.trim().trim_matches('"');
    if val.len() as u16 > quotas.max_scalar_bytes {
        return Err(IndexError::QuotaExceeded("scalar length"));
    }
    let text = if path.is_empty() {
        val.to_string()
    } else {
        alloc::format!("{path} = {val}")
    };
    push_block(
        out,
        quotas,
        ParsedBlock {
            block_kind: ParsedBlockKind::KeyValue,
            byte_start: 0,
            byte_end: 0,
            line_start: 1,
            line_end: 1,
            heading_path: path.to_string(),
            text,
        },
    )
}

fn walk_object(
    s: &str,
    path: &str,
    depth: u16,
    quotas: &IndexQuotaConfig,
    out: &mut Vec<ParsedBlock>,
    entries: &mut u32,
) -> Result<(), IndexError> {
    let inner = trim_braces(s, '{', '}')?;
    let fields = split_top_level(inner, ',')?;
    for field in fields {
        let field = field.trim();
        if field.is_empty() {
            continue;
        }
        let (k, v) = split_top_level_once(field, ':')
            .ok_or(IndexError::ParseFailed("json field"))?;
        let key = k.trim().trim_matches('"');
        if key.is_empty() {
            return Err(IndexError::ParseFailed("json key"));
        }
        let child = if path.is_empty() {
            key.to_string()
        } else {
            alloc::format!("{path}.{key}")
        };
        walk_json_like(v.trim(), &child, depth + 1, quotas, out, entries)?;
    }
    Ok(())
}

fn walk_array(
    s: &str,
    path: &str,
    depth: u16,
    quotas: &IndexQuotaConfig,
    out: &mut Vec<ParsedBlock>,
    entries: &mut u32,
) -> Result<(), IndexError> {
    let inner = trim_braces(s, '[', ']')?;
    let items = split_top_level(inner, ',')?;
    for (i, item) in items.iter().enumerate() {
        let child = alloc::format!("{path}[{i}]");
        walk_json_like(item.trim(), &child, depth + 1, quotas, out, entries)?;
    }
    Ok(())
}

fn trim_braces(s: &str, open: char, close: char) -> Result<&str, IndexError> {
    let s = s.trim();
    if !s.starts_with(open) || !s.ends_with(close) {
        return Err(IndexError::ParseFailed("json braces"));
    }
    Ok(&s[open.len_utf8()..s.len() - close.len_utf8()])
}

fn split_top_level(s: &str, sep: char) -> Result<Vec<&str>, IndexError> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut in_str = false;
    let mut escape = false;
    let mut start = 0usize;
    let bytes = s.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        let c = bytes[i] as char;
        if in_str {
            if escape {
                escape = false;
            } else if c == '\\' {
                escape = true;
            } else if c == '"' {
                in_str = false;
            }
            i += 1;
            continue;
        }
        match c {
            '"' => in_str = true,
            '{' | '[' => depth += 1,
            '}' | ']' => depth -= 1,
            ch if ch == sep && depth == 0 => {
                out.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
        if depth < 0 {
            return Err(IndexError::ParseFailed("json depth"));
        }
        i += 1;
    }
    if start <= s.len() {
        let tail = s[start..].trim();
        if !tail.is_empty() {
            out.push(&s[start..]);
        }
    }
    Ok(out)
}

fn split_top_level_once(s: &str, sep: char) -> Option<(&str, &str)> {
    let mut depth = 0i32;
    let mut in_str = false;
    let mut escape = false;
    for (i, c) in s.char_indices() {
        if in_str {
            if escape {
                escape = false;
            } else if c == '\\' {
                escape = true;
            } else if c == '"' {
                in_str = false;
            }
            continue;
        }
        match c {
            '"' => in_str = true,
            '{' | '[' => depth += 1,
            '}' | ']' => depth -= 1,
            ch if ch == sep && depth == 0 => {
                return Some((&s[..i], &s[i + ch.len_utf8()..]));
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_paths() {
        let p = JsonParser;
        let q = IndexQuotaConfig::default();
        let mut out = Vec::new();
        p.parse(r#"{"server":{"timeout":30},"theme":"dark"}"#, &q, &mut out)
            .unwrap();
        assert!(out.iter().any(|b| b.text.contains("server.timeout")));
        assert!(out.iter().any(|b| b.text.contains("theme")));
    }

    #[test]
    fn json_nesting_limit() {
        let p = JsonParser;
        let mut q = IndexQuotaConfig::default();
        q.max_parser_nesting = 2;
        let mut out = Vec::new();
        let r = p.parse(r#"{"a":{"b":{"c":1}}}"#, &q, &mut out);
        assert!(r.is_err());
    }

    #[test]
    fn toml_keys() {
        let p = TomlParser;
        let q = IndexQuotaConfig::default();
        let mut out = Vec::new();
        p.parse("[user]\nname = \"ada\"\n", &q, &mut out).unwrap();
        assert!(out.iter().any(|b| b.text.contains("user.name")));
    }

    #[test]
    fn yaml_plain_fallback() {
        let p = YamlTextParser;
        let q = IndexQuotaConfig::default();
        let mut out = Vec::new();
        p.parse("a: 1\nb: 2\n", &q, &mut out).unwrap();
        assert!(!out.is_empty());
    }
}
