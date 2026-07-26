//! Bounded CSV parser (row-oriented blocks, no schema truth inference).

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::config::PARSER_CSV;
use crate::error::IndexError;
use crate::parse::{push_block, DocumentParser, ParseSummary, ParsedBlock, ParsedBlockKind};
use crate::quotas::IndexQuotaConfig;

pub struct CsvParser;

impl DocumentParser for CsvParser {
    fn parser_id(&self) -> u32 {
        PARSER_CSV
    }
    fn parser_version(&self) -> u32 {
        1
    }
    fn supports(&self, extension: &str, _: Option<&str>) -> bool {
        extension == "csv"
    }
    fn parse(
        &self,
        input: &str,
        quotas: &IndexQuotaConfig,
        output: &mut Vec<ParsedBlock>,
    ) -> Result<ParseSummary, IndexError> {
        let before = output.len() as u32;
        let rows = parse_csv_rows(input, quotas)?;
        if rows.is_empty() {
            return Ok(ParseSummary {
                blocks: 0,
                bytes_consumed: input.len() as u64,
            });
        }
        let headers = rows[0].clone();
        let data_rows = if rows.len() > 1 { &rows[1..] } else { &[][..] };
        let mut batch = String::new();
        let mut batch_start_line = 2u32;
        let mut line = 2u32;
        let mut rows_in_batch = 0u16;

        for row in data_rows {
            if row.len() as u16 > quotas.max_csv_columns {
                return Err(IndexError::QuotaExceeded("csv columns"));
            }
            let mut parts = Vec::new();
            for (i, cell) in row.iter().enumerate() {
                if cell.len() as u16 > quotas.max_scalar_bytes {
                    return Err(IndexError::QuotaExceeded("csv field"));
                }
                let key = headers.get(i).map(|s| s.as_str()).unwrap_or("col");
                parts.push(alloc::format!("{key}={cell}"));
            }
            let row_text = parts.join("; ");
            if rows_in_batch >= quotas.max_csv_rows_per_block {
                flush_csv_batch(output, quotas, &mut batch, batch_start_line, line.saturating_sub(1))?;
                batch_start_line = line;
                rows_in_batch = 0;
            }
            if !batch.is_empty() {
                batch.push('\n');
            }
            batch.push_str(&row_text);
            rows_in_batch = rows_in_batch.saturating_add(1);
            line = line.saturating_add(1);
        }
        flush_csv_batch(output, quotas, &mut batch, batch_start_line, line.saturating_sub(1))?;

        // Also emit header as key-value meta block.
        if !headers.is_empty() {
            push_block(
                output,
                quotas,
                ParsedBlock {
                    block_kind: ParsedBlockKind::KeyValue,
                    byte_start: 0,
                    byte_end: 0,
                    line_start: 1,
                    line_end: 1,
                    heading_path: String::from("header"),
                    text: alloc::format!("header = {}", headers.join(",")),
                },
            )?;
        }

        Ok(ParseSummary {
            blocks: (output.len() as u32).saturating_sub(before),
            bytes_consumed: input.len() as u64,
        })
    }
}

fn flush_csv_batch(
    out: &mut Vec<ParsedBlock>,
    quotas: &IndexQuotaConfig,
    batch: &mut String,
    start: u32,
    end: u32,
) -> Result<(), IndexError> {
    if batch.trim().is_empty() {
        batch.clear();
        return Ok(());
    }
    push_block(
        out,
        quotas,
        ParsedBlock {
            block_kind: ParsedBlockKind::TableRow,
            byte_start: 0,
            byte_end: 0,
            line_start: start,
            line_end: end.max(start),
            heading_path: String::new(),
            text: batch.clone(),
        },
    )?;
    batch.clear();
    Ok(())
}

fn parse_csv_rows(input: &str, quotas: &IndexQuotaConfig) -> Result<Vec<Vec<String>>, IndexError> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut in_quotes = false;
    let bytes = input.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        let c = bytes[i];
        if in_quotes {
            if c == b'"' {
                if i + 1 < bytes.len() && bytes[i + 1] == b'"' {
                    field.push('"');
                    i += 2;
                    continue;
                }
                in_quotes = false;
                i += 1;
                continue;
            }
            field.push(c as char);
            i += 1;
            continue;
        }
        match c {
            b'"' => {
                in_quotes = true;
                i += 1;
            }
            b',' => {
                row.push(core::mem::take(&mut field));
                if row.len() as u16 > quotas.max_csv_columns {
                    return Err(IndexError::QuotaExceeded("csv columns"));
                }
                i += 1;
            }
            b'\n' => {
                row.push(core::mem::take(&mut field));
                rows.push(core::mem::take(&mut row));
                if rows.len() as u32 > quotas.max_blocks_per_file.saturating_mul(quotas.max_csv_rows_per_block as u32) {
                    return Err(IndexError::QuotaExceeded("csv rows"));
                }
                i += 1;
            }
            b'\r' => {
                i += 1;
            }
            _ => {
                field.push(c as char);
                i += 1;
            }
        }
    }
    if in_quotes {
        return Err(IndexError::ParseFailed("csv unclosed quote"));
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoted_fields() {
        let p = CsvParser;
        let q = IndexQuotaConfig::default();
        let mut out = Vec::new();
        p.parse("name,city\n\"Ada, Lovelace\",\"London\"\n", &q, &mut out)
            .unwrap();
        assert!(out.iter().any(|b| b.text.contains("Ada, Lovelace")));
    }
}
