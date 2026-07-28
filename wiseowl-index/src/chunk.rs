//! Deterministic, versioned document chunking.
//!
//! Chunk content digests use the strong SHA-256 identity (same algorithm as
//! document content). Stable chunk IDs fold digest bytes with FNV for a u64
//! handle — that fold is not final content proof.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use wiseowl_memory::SourceId;

use crate::config::CHUNKING_ID_V1;
use crate::digest::{digest_bytes, ContentDigest};
use crate::error::IndexError;
use crate::hash::fnv1a64;
use crate::parse::ParsedBlock;
use crate::quotas::IndexQuotaConfig;

/// Chunking profile (versioned).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkingProfile {
    pub chunking_id: u32,
    pub version: u32,
    pub target_tokens: u32,
    pub maximum_tokens: u32,
    pub maximum_bytes: u32,
    pub overlap_tokens: u16,
    pub preserve_blocks: bool,
}

impl Default for ChunkingProfile {
    fn default() -> Self {
        Self {
            chunking_id: CHUNKING_ID_V1,
            version: 1,
            target_tokens: 128,
            maximum_tokens: 256,
            maximum_bytes: 4096,
            overlap_tokens: 0,
            preserve_blocks: true,
        }
    }
}

/// One document chunk with source ranges.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentChunk {
    pub chunk_id: u64,
    pub source_id: SourceId,
    pub source_revision: u32,
    pub ordinal: u32,
    pub byte_start: u64,
    pub byte_end: u64,
    pub line_start: u32,
    pub line_end: u32,
    pub heading_path: String,
    /// Strong content digest of chunk text.
    pub content_digest: ContentDigest,
    pub text: String,
    pub overlap_from_previous: bool,
}

/// Stable chunk id from source + digest fingerprint + ordinal + versions.
pub fn stable_chunk_id(
    source_id: SourceId,
    content_digest: &ContentDigest,
    ordinal: u32,
    chunking_id: u32,
    chunking_version: u32,
    parser_id: u32,
    parser_version: u32,
) -> u64 {
    let mut buf = [0u8; 56];
    buf[0..8].copy_from_slice(&source_id.get().to_le_bytes());
    let enc = content_digest.encode();
    buf[8..43].copy_from_slice(&enc);
    buf[43..47].copy_from_slice(&ordinal.to_le_bytes());
    buf[47..51].copy_from_slice(&chunking_id.to_le_bytes());
    buf[51..55].copy_from_slice(&chunking_version.to_le_bytes());
    // last byte unused; fold parser into FNV of whole buffer + extra
    let mut full = Vec::with_capacity(64);
    full.extend_from_slice(&buf);
    full.extend_from_slice(&parser_id.to_le_bytes());
    full.extend_from_slice(&parser_version.to_le_bytes());
    fnv1a64(&full)
}

/// Chunk parsed blocks deterministically.
pub fn chunk_blocks(
    source_id: SourceId,
    source_revision: u32,
    parser_id: u32,
    parser_version: u32,
    profile: &ChunkingProfile,
    blocks: &[ParsedBlock],
    quotas: &IndexQuotaConfig,
) -> Result<Vec<DocumentChunk>, IndexError> {
    let mut chunks = Vec::new();
    let mut current_text = String::new();
    let mut current_start: Option<(u64, u32, String)> = None;
    let mut current_end: (u64, u32) = (0, 0);
    let max_bytes = profile.maximum_bytes.min(quotas.max_bytes_per_chunk) as usize;

    let flush = |chunks: &mut Vec<DocumentChunk>,
                 text: &mut String,
                 start: &mut Option<(u64, u32, String)>,
                 end: (u64, u32)|
     -> Result<(), IndexError> {
        if text.trim().is_empty() {
            text.clear();
            *start = None;
            return Ok(());
        }
        if chunks.len() as u32 >= quotas.max_chunks_per_file {
            return Err(IndexError::QuotaExceeded("chunks per file"));
        }
        let (bs, ls, hp) = start.take().unwrap_or((0, 1, String::new()));
        let content_digest = digest_bytes(text.as_bytes());
        let ordinal = chunks.len() as u32;
        let chunk_id = stable_chunk_id(
            source_id,
            &content_digest,
            ordinal,
            profile.chunking_id,
            profile.version,
            parser_id,
            parser_version,
        );
        chunks.push(DocumentChunk {
            chunk_id,
            source_id,
            source_revision,
            ordinal,
            byte_start: bs,
            byte_end: end.0,
            line_start: ls,
            line_end: end.1,
            heading_path: hp,
            content_digest,
            text: text.clone(),
            overlap_from_previous: false,
        });
        text.clear();
        Ok(())
    };

    for block in blocks {
        if block.text.trim().is_empty() {
            continue;
        }
        let block_bytes = block.text.len();
        if current_text.len().saturating_add(block_bytes) > max_bytes && !current_text.is_empty() {
            flush(
                &mut chunks,
                &mut current_text,
                &mut current_start,
                current_end,
            )?;
        }
        // Oversized single block: emit as its own chunk (truncated by max_bytes).
        if block_bytes > max_bytes {
            flush(
                &mut chunks,
                &mut current_text,
                &mut current_start,
                current_end,
            )?;
            let mut remaining = block.text.as_str();
            let mut byte_cursor = block.byte_start;
            let mut line_cursor = block.line_start;
            while !remaining.is_empty() {
                let take = max_bytes.min(remaining.len());
                // Avoid splitting mid-char
                let mut end = take;
                while end > 0 && !remaining.is_char_boundary(end) {
                    end -= 1;
                }
                if end == 0 {
                    end = remaining.chars().next().map(|c| c.len_utf8()).unwrap_or(1);
                }
                let piece = &remaining[..end];
                current_text.push_str(piece);
                current_start = Some((byte_cursor, line_cursor, block.heading_path.clone()));
                current_end = (byte_cursor.saturating_add(end as u64), block.line_end);
                flush(
                    &mut chunks,
                    &mut current_text,
                    &mut current_start,
                    current_end,
                )?;
                remaining = &remaining[end..];
                byte_cursor = byte_cursor.saturating_add(end as u64);
                line_cursor = block.line_end;
            }
            continue;
        }
        if current_start.is_none() {
            current_start = Some((
                block.byte_start,
                block.line_start,
                block.heading_path.clone(),
            ));
        } else if !current_text.is_empty() {
            current_text.push('\n');
        }
        current_text.push_str(&block.text);
        current_end = (block.byte_end, block.line_end);
    }
    flush(
        &mut chunks,
        &mut current_text,
        &mut current_start,
        current_end,
    )?;
    Ok(chunks)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::{ParsedBlock, ParsedBlockKind};

    #[test]
    fn deterministic_chunk_ids() {
        let blocks = vec![ParsedBlock {
            block_kind: ParsedBlockKind::Paragraph,
            byte_start: 0,
            byte_end: 11,
            line_start: 1,
            line_end: 1,
            heading_path: String::new(),
            text: String::from("hello world"),
        }];
        let q = IndexQuotaConfig::default();
        let sid = SourceId::from_raw_unchecked(1);
        let a = chunk_blocks(sid, 1, 1, 1, &ChunkingProfile::default(), &blocks, &q).unwrap();
        let b = chunk_blocks(sid, 1, 1, 1, &ChunkingProfile::default(), &blocks, &q).unwrap();
        assert_eq!(a[0].chunk_id, b[0].chunk_id);
        assert_eq!(a[0].content_digest, b[0].content_digest);
        assert!(a[0].content_digest.is_set());
    }
}
